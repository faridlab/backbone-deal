//! Golden-rule oracle for the stage-driven opportunity lifecycle (the derived won/lost rules).
//!
//! Deal-only: proves the derived-status composition against real Postgres — won forces
//! probability 100, plain moves preserve manual probabilities, lost = archive + 0, date_closed
//! stamps once, the win verb picks the right won stage and hands off at most once, and the DB
//! CHECKs backstop raw writers.
//!
//! Tenancy (ADR-0029): the module is tenant-agnostic — its services carry no tenant argument and
//! its statements ride the ambient org scope when one is bound. These goldens therefore run each
//! case in its OWN throwaway database (the stages table is global per database now — there is no
//! per-company key to isolate by), binding an org request scope around the verbs that mint
//! company-keyed seams (the selling handoff and the outcome events). The row-level fence proofs
//! moved to the COMPOSING service: on a module-local database no decorator is mounted, so a
//! fence's visibility rules cannot be observed here.
//!
//! DB: DATABASE_URL wins, else the module's local scratch postgres. Every golden creates its own
//! database from the repo's `migrations/` chain and drops it when done (and before it starts, so
//! re-runs converge).

use rust_decimal::Decimal;
use sqlx::{Executor, PgPool};
use uuid::Uuid;

use backbone_deal::application::service::deal_ports::{
    CrmRejected, QuotationAck, QuotationFromOpp, SellingPort,
};
use backbone_deal::application::service::deal_write_service::{DealError, DealWriteService};
use backbone_deal::domain::event::{DealEvent, DealEventSink};
use backbone_deal::infrastructure::persistence::{
    NewOpportunityRow, NewOppItemRow, OpportunityItemRepository, OpportunityRepository,
};
use backbone_orm::org_scope::{OrgScope, with_org_request_scope};

fn d(s: &str) -> Decimal {
    Decimal::from_str_exact(s).unwrap()
}

fn admin_url() -> String {
    // DATABASE_URL points at the module's scratch postgres; the admin connection swaps the path
    // for the maintenance database (createdb/dropdb need it).
    let url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgresql://postgres:postgres@127.0.0.1:5433/deal_stage_goldens".into());
    let (prefix, _) = url.rsplit_once('/').expect("DATABASE_URL with a database path");
    format!("{prefix}/postgres")
}

/// Create (or reuse-reset) a throwaway database with the full migration chain applied, for one
/// golden. Stages are global per database, so per-test databases are the isolation key.
async fn fresh_db(name: &str) -> PgPool {
    let admin = PgPool::connect(&admin_url()).await.expect("connect maintenance DB");
    admin
        .execute(format!("DROP DATABASE IF EXISTS {name}").as_str())
        .await
        .expect("drop scratch DB");
    admin
        .execute(format!("CREATE DATABASE {name}").as_str())
        .await
        .expect("create scratch DB");
    let url = admin_url();
    let (prefix, _) = url.rsplit_once('/').unwrap();
    let pool = PgPool::connect(&format!("{prefix}/{name}")).await.expect("connect scratch DB");

    let mut entries: Vec<(String, String)> = std::fs::read_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/migrations"))
        .expect("read migrations dir")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.extension().is_some_and(|x| x == "sql")
                && p.file_name().is_some_and(|n| n.to_string_lossy().ends_with(".up.sql"))
        })
        .filter_map(|p| Some((p.file_name()?.to_string_lossy().to_string(), std::fs::read_to_string(&p).ok()?)))
        .collect();
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    for (name, sql) in &entries {
        sqlx::raw_sql(sql)
            .execute(&pool)
            .await
            .unwrap_or_else(|e| panic!("applying {name}: {e}"));
    }
    pool
}

async fn drop_db(name: &str) {
    let admin = PgPool::connect(&admin_url()).await.expect("connect maintenance DB");
    admin
        .execute(format!("DROP DATABASE IF EXISTS {name}").as_str())
        .await
        .expect("drop scratch DB");
}

/// A no-op selling port + a recording event sink.
struct FakeSelling {
    quotation: Uuid,
    calls: std::sync::Mutex<usize>,
}
#[async_trait::async_trait]
impl SellingPort for FakeSelling {
    async fn create_quotation(&self, _req: &QuotationFromOpp) -> Result<QuotationAck, CrmRejected> {
        *self.calls.lock().unwrap() += 1;
        Ok(QuotationAck { quotation_id: self.quotation })
    }
}
#[derive(Default)]
struct RecordingSink(std::sync::Mutex<Vec<&'static str>>);
impl DealEventSink for RecordingSink {
    fn publish(&self, event: &DealEvent) {
        let tag = match event {
            DealEvent::OpportunityWon(_) => "won",
            DealEvent::OpportunityLost(_) => "lost",
        };
        self.0.lock().unwrap().push(tag);
    }
}

/// The default pipeline, seeded through the repository's lazy ensure (the module-side half of
/// the seeding contract — the migration's historical company-keyed seeding is covered by the
/// migration goldens).
async fn seed_default_stages(pool: &PgPool) {
    let repo = OpportunityRepository::new(pool.clone());
    let mut conn = pool.acquire().await.unwrap();
    repo.ensure_default_stages(&mut conn).await.unwrap();
}

/// The id of the lowest-sequence stage carrying `code` — how the goldens address stages now
/// that ids are database-minted (no deterministic formula to recompute).
async fn stage_id_by_code(pool: &PgPool, code: &str) -> Uuid {
    sqlx::query_scalar("SELECT id FROM deal.stages WHERE code=$1 ORDER BY sequence ASC LIMIT 1")
        .bind(code)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn new_opportunity(pool: &PgPool, name: &str, probability: Decimal) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO deal.opportunities
             (id, opportunity_name, lead_id, party_id, currency, expected_amount, stage_id,
              probability, status, active, date_last_stage_update, metadata)
           VALUES ($1, $2, $3, $4, 'IDR', 1000, $5, $6, 'open'::opportunity_status, TRUE, NOW(),
                   '{"created_at":null,"updated_at":null,"deleted_at":null,"created_by":null,"updated_by":null,"deleted_by":null}'::jsonb)"#,
    )
    .bind(id)
    .bind(name)
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(stage_id_by_code(pool, "qualification").await)
    .bind(probability)
    .execute(pool)
    .await
    .unwrap();
    id
}

/// One deal's world: the default pipeline plus one open opportunity on the entry stage.
async fn seed_world(pool: &PgPool, probability: Decimal) -> Uuid {
    seed_default_stages(pool).await;
    new_opportunity(pool, "Golden deal", probability).await
}

/// Bind a request org scope around `f` — the composing-service posture the win/lose verbs need
/// (their company-keyed seams read the scope's legacy company, fail-closed).
async fn scoped<R, F>(pool: &PgPool, f: F) -> R
where
    F: std::future::Future<Output = R>,
{
    let unit = Uuid::new_v4();
    with_org_request_scope(pool, OrgScope::for_company_unit(unit), f)
        .await
        .expect("bind org request scope")
}

/// The raw row the assertions read: everything the derived-status rules touch.
#[derive(Debug, sqlx::FromRow)]
struct OppRow {
    status: String,
    probability: Decimal,
    active: bool,
    stage_id: Uuid,
    quotation_id: Option<Uuid>,
    date_last_stage_update: Option<chrono::DateTime<chrono::Utc>>,
    date_closed: Option<chrono::DateTime<chrono::Utc>>,
}
async fn read_opp(pool: &PgPool, id: Uuid) -> OppRow {
    sqlx::query_as::<_, OppRow>(
        r#"SELECT status::text AS status, probability, active, stage_id, quotation_id,
                  date_last_stage_update, date_closed
           FROM deal.opportunities WHERE id=$1"#,
    )
    .bind(id)
    .fetch_one(pool)
    .await
    .unwrap()
}

/// One quotable line on the deal (the win verb requires at least one).
async fn add_line(pool: &PgPool, opportunity: Uuid) {
    let items = OpportunityItemRepository::new(pool.clone());
    let mut conn = pool.acquire().await.unwrap();
    items
        .insert_item(
            &mut conn,
            &NewOppItemRow {
                id: Uuid::new_v4(),
                opportunity_id: opportunity,
                item_id: Uuid::new_v4(),
                description: None,
                quantity: d("2"),
                rate: d("500"),
                amount: d("1000"),
            },
        )
        .await
        .unwrap();
}

/// G1: entering an is_won stage FORCES probability 100 (a body value cannot override it) and
/// derives status=won — with the quotation handoff still untouched (NULL).
#[tokio::test]
async fn golden_stage_to_won_forces_probability_100() {
    let pool = fresh_db("deal_golden_g1").await;
    let opp = seed_world(&pool, d("30")).await;
    let svc = DealWriteService::new(pool.clone());
    let won = stage_id_by_code(&pool, "won").await;

    // Leg 1: probability omitted.
    let m = svc.advance_stage(opp, won, None).await.unwrap();
    assert_eq!(m.status, "won");
    assert_eq!(m.probability, Decimal::from(100));
    let row = read_opp(&pool, opp).await;
    assert_eq!(row.status, "won");
    assert_eq!(row.probability, Decimal::from(100));
    assert!(row.date_closed.is_some(), "entering a won stage stamps date_closed");
    assert!(row.quotation_id.is_none(), "drag-to-won sets no quotation");

    // Leg 2: a body value that tries to keep it below 100 — the won stage still forces 100.
    let opp2 = new_opportunity(&pool, "Golden deal II", d("30")).await;
    let m2 = svc.advance_stage(opp2, won, Some(d("55"))).await.unwrap();
    assert_eq!(m2.status, "won");
    assert_eq!(m2.probability, Decimal::from(100));

    pool.close().await;
    drop_db("deal_golden_g1").await;
}

/// G2: a plain move NEVER clobbers a manual probability, and the stage's probability_hint is
/// advisory only (it is not auto-applied).
#[tokio::test]
async fn golden_plain_move_preserves_manual_probability() {
    let pool = fresh_db("deal_golden_g2").await;
    let opp = seed_world(&pool, d("55")).await;
    let svc = DealWriteService::new(pool.clone());

    let m = svc
        .advance_stage(opp, stage_id_by_code(&pool, "negotiation").await, None)
        .await
        .unwrap();
    assert_eq!(m.status, "open");
    assert_eq!(m.probability, d("55"), "plain move preserves the manual probability");
    let row = read_opp(&pool, opp).await;
    assert_eq!(row.probability, d("55"));
    // The negotiation stage's seeded hint is 70 — explicitly NOT applied.
    assert_ne!(row.probability, d("70"));
    assert!(row.date_closed.is_none());
    assert!(row.date_last_stage_update.is_some(), "an actual stage change stamps the move");

    pool.close().await;
    drop_db("deal_golden_g2").await;
}

/// G3: explicit probability edits — same-stage edit allowed, out-of-range rejected, and an edit
/// to exactly 100 on a NON-won stage stamps date_closed while the status stays open.
#[tokio::test]
async fn golden_manual_probability_edit_rules() {
    let pool = fresh_db("deal_golden_g3").await;
    let opp = seed_world(&pool, d("10")).await;
    let svc = DealWriteService::new(pool.clone());
    let same = stage_id_by_code(&pool, "qualification").await;

    // Same-stage edit: allowed, applies the value, does not count as a stage change
    // (date_last_stage_update keeps its seeded value — no NEW stamp).
    let before = read_opp(&pool, opp).await;
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    let m = svc.advance_stage(opp, same, Some(d("99"))).await.unwrap();
    assert_eq!(m.probability, d("99"));
    let after = read_opp(&pool, opp).await;
    assert_eq!(after.probability, d("99"));
    assert_eq!(after.date_last_stage_update, before.date_last_stage_update,
        "a same-stage probability edit is not a stage change");

    // Out-of-range: rejected.
    let err = svc.advance_stage(opp, same, Some(d("101"))).await.unwrap_err();
    assert!(matches!(err, DealError::Invalid(_)), "101 must be rejected: {err:?}");
    assert_eq!(err.http_status(), 422);

    // Exactly 100 on a non-won stage: date_closed stamps, status stays open (won needs a won
    // stage — probability alone is not a close).
    let m = svc.advance_stage(opp, same, Some(d("100"))).await.unwrap();
    assert_eq!(m.status, "open");
    let row = read_opp(&pool, opp).await;
    assert_eq!(row.status, "open");
    assert!(row.date_closed.is_some(), "reaching probability 100 stamps date_closed");

    pool.close().await;
    drop_db("deal_golden_g3").await;
}

/// G4: lose archives (active=false) + zeroes probability + keeps the stage, is terminal, and
/// carries the reason/competitor.
#[tokio::test]
async fn golden_lose_archives_and_zeroes() {
    let pool = fresh_db("deal_golden_g4").await;
    let opp = seed_world(&pool, d("40")).await;
    let svc = DealWriteService::new(pool.clone());
    let sink = RecordingSink::default();

    // Park the deal on proposal first so "the stage is kept" is observable.
    svc.advance_stage(opp, stage_id_by_code(&pool, "proposal").await, None)
        .await
        .unwrap();
    let stage_before = read_opp(&pool, opp).await.stage_id;

    scoped(&pool, svc.lose_opportunity(opp, Some("price".into()), Some("acme".into()), &sink))
        .await
        .unwrap();

    let row = read_opp(&pool, opp).await;
    assert_eq!(row.status, "lost");
    assert!(!row.active, "lost is archived");
    assert_eq!(row.probability, Decimal::ZERO);
    assert_eq!(row.stage_id, stage_before, "the row remembers where it died");
    assert!(row.date_closed.is_some());
    assert_eq!(sink.0.lock().unwrap().as_slice(), ["lost"]);
    let (reason, competitor): (Option<String>, Option<String>) =
        sqlx::query_as("SELECT lost_reason, competitor FROM deal.opportunities WHERE id=$1")
            .bind(opp)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(reason.as_deref(), Some("price"));
    assert_eq!(competitor.as_deref(), Some("acme"));

    // Terminal: a second lose and any stage move are both rejected.
    let err = scoped(&pool, svc.lose_opportunity(opp, None, None, &sink)).await.unwrap_err();
    assert!(matches!(err, DealError::InvalidState(_)));
    assert_eq!(err.http_status(), 422);
    let err = svc
        .advance_stage(opp, stage_id_by_code(&pool, "closing").await, None)
        .await
        .unwrap_err();
    assert!(matches!(err, DealError::InvalidState(_)));

    pool.close().await;
    drop_db("deal_golden_g4").await;
}

/// G5: a won→won slide never rewrites date_closed (it stamps only on first close), while the
/// stage-change stamp does move.
#[tokio::test]
async fn golden_won_to_won_preserves_date_closed() {
    let pool = fresh_db("deal_golden_g5").await;
    seed_default_stages(&pool).await;
    // A second won stage, sequenced after the seeded one.
    sqlx::query(
        r#"INSERT INTO deal.stages (code, name, sequence, is_won, probability_hint, active, metadata)
           VALUES ('won-repeat', 'Won again', 70, TRUE, 100, TRUE, '{}'::jsonb)"#,
    )
    .execute(&pool)
    .await
    .unwrap();

    let opportunity = new_opportunity(&pool, "Won-won slide", d("60")).await;
    add_line(&pool, opportunity).await;
    let svc = DealWriteService::new(pool.clone());
    let selling = FakeSelling { quotation: Uuid::new_v4(), calls: std::sync::Mutex::new(0) };
    let sink = RecordingSink::default();

    scoped(&pool, svc.win_opportunity(opportunity, &selling, &sink)).await.unwrap();
    let after_win = read_opp(&pool, opportunity).await;
    assert_eq!(after_win.status, "won");
    assert_eq!(after_win.stage_id, stage_id_by_code(&pool, "won").await,
        "win lands on the lowest-sequence won stage");

    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    svc.advance_stage(opportunity, stage_id_by_code(&pool, "won-repeat").await, None)
        .await
        .unwrap();
    let after_slide = read_opp(&pool, opportunity).await;
    assert_eq!(after_slide.status, "won");
    assert_eq!(after_slide.probability, Decimal::from(100), "the second won stage also forces 100");
    assert_eq!(after_slide.date_closed, after_win.date_closed, "won→won never rewrites date_closed");
    assert!(after_slide.date_last_stage_update > after_win.date_last_stage_update,
        "the actual stage change still stamps date_last_stage_update");
    assert_eq!(after_slide.quotation_id, Some(selling.quotation));

    pool.close().await;
    drop_db("deal_golden_g5").await;
}

/// G6: the win verb picks the LOWEST-sequence active won stage, hands off exactly once (the
/// replay returns `already=true` with the same quotation), and a deployment with no selectable
/// won stage is rejected.
#[tokio::test]
async fn golden_win_picks_lowest_won_sequence_and_is_idempotent() {
    let pool = fresh_db("deal_golden_g6").await;
    seed_default_stages(&pool).await;
    // A higher-sequence won stage the verb must NOT pick.
    sqlx::query(
        r#"INSERT INTO deal.stages (code, name, sequence, is_won, probability_hint, active, metadata)
           VALUES ('won-later', 'Won later', 90, TRUE, 100, TRUE, '{}'::jsonb)"#,
    )
    .execute(&pool)
    .await
    .unwrap();

    let opportunity = new_opportunity(&pool, "Win pick", d("50")).await;
    // The win needs a party (seeded by new_opportunity) + at least one line.
    add_line(&pool, opportunity).await;

    let svc = DealWriteService::new(pool.clone());
    let selling = FakeSelling { quotation: Uuid::new_v4(), calls: std::sync::Mutex::new(0) };
    let sink = RecordingSink::default();

    let out = scoped(&pool, svc.win_opportunity(opportunity, &selling, &sink)).await.unwrap();
    assert!(!out.already);
    let row = read_opp(&pool, opportunity).await;
    assert_eq!(row.status, "won");
    assert_eq!(row.stage_id, stage_id_by_code(&pool, "won").await, "seq 60 beats seq 90");
    assert_eq!(row.probability, Decimal::from(100));
    assert_eq!(row.quotation_id, Some(selling.quotation));
    assert_eq!(*selling.calls.lock().unwrap(), 1);
    assert_eq!(sink.0.lock().unwrap().as_slice(), ["won"]);

    // Replay: idempotent — same quotation, no second handoff, no second event.
    let replay = scoped(&pool, svc.win_opportunity(opportunity, &selling, &sink)).await.unwrap();
    assert!(replay.already);
    assert_eq!(replay.quotation_id, selling.quotation);
    assert_eq!(*selling.calls.lock().unwrap(), 1);
    assert_eq!(sink.0.lock().unwrap().len(), 1);

    // A deployment whose only won stage is unselectable cannot win.
    sqlx::query("UPDATE deal.stages SET active=FALSE WHERE is_won").execute(&pool).await.unwrap();
    let stuck = new_opportunity(&pool, "No won stage", d("10")).await;
    let err = scoped(&pool, svc.win_opportunity(stuck, &selling, &sink)).await.unwrap_err();
    assert!(matches!(err, DealError::InvalidState(_)), "no active won stage: {err:?}");
    assert_eq!(err.http_status(), 422);

    pool.close().await;
    drop_db("deal_golden_g6").await;
}

/// G7: the ADR-0015 DB backstops — a raw writer cannot hand-set status='won' on an inconsistent
/// row, nor write probability > 100. Status is reachable only through the gated statements.
#[tokio::test]
async fn golden_status_not_hand_settable() {
    let pool = fresh_db("deal_golden_g7").await;
    let opp = seed_world(&pool, d("30")).await;

    let won_violation = sqlx::query("UPDATE deal.opportunities SET status='won'::opportunity_status WHERE id=$1")
        .bind(opp)
        .execute(&pool)
        .await
        .unwrap_err();
    assert!(
        won_violation.to_string().contains("opportunities_won_shape"),
        "hand-set won must hit the shape CHECK: {won_violation}"
    );

    let range_violation = sqlx::query("UPDATE deal.opportunities SET probability=101 WHERE id=$1")
        .bind(opp)
        .execute(&pool)
        .await
        .unwrap_err();
    assert!(
        range_violation.to_string().contains("opportunities_probability_range"),
        "probability 101 must hit the range CHECK: {range_violation}"
    );

    // And the lose shape: lost without archiving is equally unreachable for a raw writer.
    let lost_violation = sqlx::query("UPDATE deal.opportunities SET status='lost'::opportunity_status WHERE id=$1")
        .bind(opp)
        .execute(&pool)
        .await
        .unwrap_err();
    assert!(
        lost_violation.to_string().contains("opportunities_lost_shape"),
        "hand-set lost must hit the shape CHECK: {lost_violation}"
    );

    pool.close().await;
    drop_db("deal_golden_g7").await;
}

/// G8: seed convergence — the lazy ensure is idempotent: a second run over a database already
/// holding the default set inserts nothing, and the ids of every stage stay stable across runs.
#[tokio::test]
async fn golden_seed_converges() {
    let pool = fresh_db("deal_golden_g8").await;

    seed_default_stages(&pool).await; // first ensure
    let before: Vec<(String, Uuid)> =
        sqlx::query_as("SELECT code, id FROM deal.stages ORDER BY sequence").fetch_all(&pool).await.unwrap();
    assert_eq!(before.len(), 6, "the ensure seeds the six default stages");

    seed_default_stages(&pool).await; // second ensure — must add nothing
    let after: Vec<(String, Uuid)> =
        sqlx::query_as("SELECT code, id FROM deal.stages ORDER BY sequence").fetch_all(&pool).await.unwrap();
    assert_eq!(after, before, "the ensure over a seeded database adds nothing and keeps ids");

    pool.close().await;
    drop_db("deal_golden_g8").await;
}

/// G9: lazy defaults — a database with no stages yet gets the default set at first opportunity
/// insert, and the deal enters at the qualification stage.
#[tokio::test]
async fn golden_lazy_defaults_on_first_opportunity() {
    let pool = fresh_db("deal_golden_g9").await;
    let n_before: i64 = sqlx::query_scalar("SELECT count(*) FROM deal.stages")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n_before, 0, "a fresh database starts with no stages");

    let repo = OpportunityRepository::new(pool.clone());
    let id = Uuid::new_v4();
    let mut conn = pool.acquire().await.unwrap();
    repo.insert_opportunity(
        &mut conn,
        &NewOpportunityRow {
            id,
            opportunity_name: "First deal",
            lead_id: Uuid::new_v4(),
            party_id: None,
            campaign_id: None,
            currency: "IDR",
            expected_amount: d("100"),
            expected_close_date: None,
        },
    )
    .await
    .unwrap();
    drop(conn);

    let n_after: i64 = sqlx::query_scalar("SELECT count(*) FROM deal.stages")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n_after, 6, "the insert lazily seeded the default set");

    let row = read_opp(&pool, id).await;
    assert_eq!(row.stage_id, stage_id_by_code(&pool, "qualification").await, "new deals enter at qualification");
    assert_eq!(row.status, "open");
    assert_eq!(row.probability, Decimal::ZERO);
    assert!(row.active);
    assert!(row.date_last_stage_update.is_some(), "the entry stamp is set at insert");

    pool.close().await;
    drop_db("deal_golden_g9").await;
}

/// G10: the move gates — a lost deal cannot move, an unknown stage id is a 404-shaped
/// stage_not_found, and an inactive stage is a 422. (The row-level fence leg lives with the
/// COMPOSING service since ADR-0029: a module-local database mounts no decorator, so a fence's
/// cross-tenant visibility rules cannot be observed here.)
#[tokio::test]
async fn golden_advance_gates() {
    let pool = fresh_db("deal_golden_g10").await;
    let a = seed_world(&pool, d("20")).await;
    let b = seed_world(&pool, d("20")).await;
    let svc = DealWriteService::new(pool.clone());

    // Unknown stage id: 404-shaped named error.
    let err = svc.advance_stage(a, Uuid::new_v4(), None).await.unwrap_err();
    assert!(matches!(err, DealError::StageNotFound(_)), "{err:?}");
    assert_eq!(err.http_status(), 404);

    // Inactive stage target: 422.
    sqlx::query(
        r#"INSERT INTO deal.stages (code, name, sequence, is_won, probability_hint, active, metadata)
           VALUES ('frozen', 'Frozen', 55, FALSE, 0, FALSE, '{}'::jsonb)"#,
    )
    .execute(&pool)
    .await
    .unwrap();
    let frozen: Uuid = sqlx::query_scalar("SELECT id FROM deal.stages WHERE code='frozen' LIMIT 1")
        .fetch_one(&pool)
        .await
        .unwrap();
    let err = svc.advance_stage(a, frozen, None).await.unwrap_err();
    assert!(matches!(err, DealError::StageInactive(_)), "{err:?}");
    assert_eq!(err.http_status(), 422);

    // Lost is terminal (G4 covers the full lose golden; here the gate leg).
    scoped(&pool, svc.lose_opportunity(b, None, None, &RecordingSink::default())).await.unwrap();
    let err = svc
        .advance_stage(b, stage_id_by_code(&pool, "closing").await, None)
        .await
        .unwrap_err();
    assert!(matches!(err, DealError::InvalidState(_)));

    pool.close().await;
    drop_db("deal_golden_g10").await;
}
