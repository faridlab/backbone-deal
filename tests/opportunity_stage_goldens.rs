//! Golden-rule oracle for the stage-driven opportunity lifecycle (the derived won/lost rules).
//!
//! Deal-only: proves the derived-status composition against real Postgres (deal.* schema, full
//! migration chain applied) — won forces probability 100, plain moves preserve manual
//! probabilities, lost = archive + 0, date_closed stamps once, the win verb picks the right won
//! stage and hands off at most once, the DB CHECKs backstop raw writers, and the stage seed
//! converges on deterministic uuid5 ids.
//!
//! Writes go through the write service (wrapped in `with_company_scope`, the request posture);
//! assertions read raw over the same pool. The cross-tenant fence leg runs under `SET ROLE` to a
//! plain non-superuser (the suite connects as the DB owner/superuser, whom RLS can never bind —
//! the composing-app posture is the only way the fence is observable).
//!
//! DB: DATABASE_URL wins, else the module's local scratch DB on the metaphora dev postgres —
//! created automatically (inside the shared container) with the full migration chain applied if
//! missing. Fresh random company ids per test so parallel runs never collide.

use rust_decimal::Decimal;
use sqlx::{Acquire, Executor, PgPool};
use uuid::Uuid;

use backbone_deal::application::service::deal_ports::{
    CrmRejected, QuotationAck, QuotationFromOpp, SellingPort,
};
use backbone_deal::application::service::deal_write_service::{DealError, DealWriteService};
use backbone_deal::domain::event::{DealEvent, DealEventSink};
use backbone_deal::infrastructure::persistence::{
    NewOpportunityRow, NewOppItemRow, OpportunityItemRepository, OpportunityRepository,
    DEFAULT_STAGES,
};

fn d(s: &str) -> Decimal {
    Decimal::from_str_exact(s).unwrap()
}

/// Provisioned once per test process: if the scratch DB is missing it is created inside the
/// container (via the maintenance-DB connection) and the repo's full migration chain is applied —
/// so the suite converges from an empty container and leaves nothing that has to be kept alive.
static PROVISION: tokio::sync::OnceCell<()> = tokio::sync::OnceCell::const_new();

async fn pool() -> PgPool {
    PROVISION.get_or_init(provision_if_missing).await;
    let url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgresql://serpa:serpa_dev_password@127.0.0.1:5432/deal_stage_goldens".into());
    PgPool::connect(&url).await.expect("connect DB")
}

async fn provision_if_missing() {
    let url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgresql://serpa:serpa_dev_password@127.0.0.1:5432/deal_stage_goldens".into());
    if PgPool::connect(&url).await.is_ok() {
        return; // already there (a previous run or a hand-applied DB)
    }
    let (prefix, db) = url.rsplit_once('/').expect("DATABASE_URL with a database path");
    let admin = PgPool::connect(&format!("{prefix}/serpa")).await.expect("connect maintenance DB");
    admin
        .execute(format!("CREATE DATABASE {db}").as_str())
        .await
        .expect("create scratch DB");
    let fresh = PgPool::connect(&url).await.expect("connect fresh scratch DB");
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
            .execute(&fresh)
            .await
            .unwrap_or_else(|e| panic!("applying {name}: {e}"));
    }
    fresh.close().await;
}

/// The deterministic default-stage id — both the seeding SQL and the lazy ensure compute exactly
/// this (uuid5 over URL namespace of "deal-stage:{company}:{code}").
fn stage_id_for(company: Uuid, code: &str) -> Uuid {
    Uuid::new_v5(&Uuid::NAMESPACE_URL, format!("deal-stage:{company}:{code}").as_bytes())
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

/// One company's world, seeded the way the reshape migration seeds (SQL-side uuid5 ids over the
/// six default stages) plus one open opportunity sitting on the entry stage.
struct World {
    company: Uuid,
    opportunity: Uuid,
}

/// Insert the six default stages using the DATABASE-side uuid5 (exactly the migration's seeding
/// expression) — the convergence golden then proves the Rust-side ensure computes the same ids.
async fn seed_default_stages_sql(pool: &PgPool, company: Uuid) {
    for (code, name, sequence, is_won, hint) in DEFAULT_STAGES {
        sqlx::query(
            r#"INSERT INTO deal.stages (id, company_id, code, name, sequence, is_won, probability_hint, active, metadata)
               VALUES (uuid_generate_v5(uuid_ns_url(), $1), $2, $3, $4, $5, $6, $7, TRUE,
                       '{"created_at":null,"updated_at":null,"deleted_at":null,"created_by":null,"updated_by":null,"deleted_by":null}'::jsonb)
               ON CONFLICT (company_id, code) DO NOTHING"#,
        )
        .bind(format!("deal-stage:{company}:{code}"))
        .bind(company)
        .bind(code)
        .bind(name)
        .bind(sequence)
        .bind(is_won)
        .bind(Decimal::from(hint))
        .execute(pool)
        .await
        .unwrap();
    }
}

async fn new_opportunity(pool: &PgPool, company: Uuid, name: &str, probability: Decimal) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO deal.opportunities
             (id, company_id, opportunity_name, lead_id, party_id, currency, expected_amount, stage_id,
              probability, status, active, date_last_stage_update, metadata)
           VALUES ($1, $2, $3, $4, $5, 'IDR', 1000, $6, $7, 'open'::opportunity_status, TRUE, NOW(),
                   '{"created_at":null,"updated_at":null,"deleted_at":null,"created_by":null,"updated_by":null,"deleted_by":null}'::jsonb)"#,
    )
    .bind(id)
    .bind(company)
    .bind(name)
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(stage_id_for(company, "qualification"))
    .bind(probability)
    .execute(pool)
    .await
    .unwrap();
    id
}

async fn seed_world(pool: &PgPool, probability: Decimal) -> World {
    let company = Uuid::new_v4();
    seed_default_stages_sql(pool, company).await;
    let opportunity = new_opportunity(pool, company, "Golden deal", probability).await;
    World { company, opportunity }
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
async fn add_line(pool: &PgPool, company: Uuid, opportunity: Uuid) {
    let items = OpportunityItemRepository::new(pool.clone());
    let mut conn = pool.acquire().await.unwrap();
    items
        .insert_item(
            &mut conn,
            &NewOppItemRow {
                id: Uuid::new_v4(),
                opportunity_id: opportunity,
                company_id: company,
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
    let pool = pool().await;
    let w = seed_world(&pool, d("30")).await;
    let svc = DealWriteService::new(pool.clone());
    let won = stage_id_for(w.company, "won");

    // Leg 1: probability omitted.
    let m = backbone_orm::company_scope::with_company_scope(
        Some(w.company),
        svc.advance_stage(w.opportunity, won, None),
    )
    .await
    .unwrap();
    assert_eq!(m.status, "won");
    assert_eq!(m.probability, Decimal::from(100));
    let row = read_opp(&pool, w.opportunity).await;
    assert_eq!(row.status, "won");
    assert_eq!(row.probability, Decimal::from(100));
    assert!(row.date_closed.is_some(), "entering a won stage stamps date_closed");
    assert!(row.quotation_id.is_none(), "drag-to-won sets no quotation");

    // Leg 2: a body value that tries to keep it below 100 — the won stage still forces 100.
    let w2 = seed_world(&pool, d("30")).await;
    let m2 = backbone_orm::company_scope::with_company_scope(
        Some(w2.company),
        svc.advance_stage(w2.opportunity, stage_id_for(w2.company, "won"), Some(d("55"))),
    )
    .await
    .unwrap();
    assert_eq!(m2.status, "won");
    assert_eq!(m2.probability, Decimal::from(100));
}

/// G2: a plain move NEVER clobbers a manual probability, and the stage's probability_hint is
/// advisory only (it is not auto-applied).
#[tokio::test]
async fn golden_plain_move_preserves_manual_probability() {
    let pool = pool().await;
    let w = seed_world(&pool, d("55")).await;
    let svc = DealWriteService::new(pool.clone());

    let m = backbone_orm::company_scope::with_company_scope(
        Some(w.company),
        svc.advance_stage(w.opportunity, stage_id_for(w.company, "negotiation"), None),
    )
    .await
    .unwrap();
    assert_eq!(m.status, "open");
    assert_eq!(m.probability, d("55"), "plain move preserves the manual probability");
    let row = read_opp(&pool, w.opportunity).await;
    assert_eq!(row.probability, d("55"));
    // The negotiation stage's seeded hint is 70 — explicitly NOT applied.
    assert_ne!(row.probability, d("70"));
    assert!(row.date_closed.is_none());
    assert!(row.date_last_stage_update.is_some(), "an actual stage change stamps the move");
}

/// G3: explicit probability edits — same-stage edit allowed, out-of-range rejected, and an edit
/// to exactly 100 on a NON-won stage stamps date_closed while the status stays open.
#[tokio::test]
async fn golden_manual_probability_edit_rules() {
    let pool = pool().await;
    let w = seed_world(&pool, d("10")).await;
    let svc = DealWriteService::new(pool.clone());
    let same = stage_id_for(w.company, "qualification");

    // Same-stage edit: allowed, applies the value, does not count as a stage change
    // (date_last_stage_update keeps its seeded value — no NEW stamp).
    let before = read_opp(&pool, w.opportunity).await;
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    let m = backbone_orm::company_scope::with_company_scope(
        Some(w.company),
        svc.advance_stage(w.opportunity, same, Some(d("99"))),
    )
    .await
    .unwrap();
    assert_eq!(m.probability, d("99"));
    let after = read_opp(&pool, w.opportunity).await;
    assert_eq!(after.probability, d("99"));
    assert_eq!(after.date_last_stage_update, before.date_last_stage_update,
        "a same-stage probability edit is not a stage change");

    // Out-of-range: rejected.
    let err = backbone_orm::company_scope::with_company_scope(
        Some(w.company),
        svc.advance_stage(w.opportunity, same, Some(d("101"))),
    )
    .await
    .unwrap_err();
    assert!(matches!(err, DealError::Invalid(_)), "101 must be rejected: {err:?}");
    assert_eq!(err.http_status(), 422);

    // Exactly 100 on a non-won stage: date_closed stamps, status stays open (won needs a won
    // stage — probability alone is not a close).
    let m = backbone_orm::company_scope::with_company_scope(
        Some(w.company),
        svc.advance_stage(w.opportunity, same, Some(d("100"))),
    )
    .await
    .unwrap();
    assert_eq!(m.status, "open");
    let row = read_opp(&pool, w.opportunity).await;
    assert_eq!(row.status, "open");
    assert!(row.date_closed.is_some(), "reaching probability 100 stamps date_closed");
}

/// G4: lose archives (active=false) + zeroes probability + keeps the stage, is terminal, and
/// carries the reason/competitor.
#[tokio::test]
async fn golden_lose_archives_and_zeroes() {
    let pool = pool().await;
    let w = seed_world(&pool, d("40")).await;
    let svc = DealWriteService::new(pool.clone());
    let sink = RecordingSink::default();

    // Park the deal on proposal first so "the stage is kept" is observable.
    backbone_orm::company_scope::with_company_scope(
        Some(w.company),
        svc.advance_stage(w.opportunity, stage_id_for(w.company, "proposal"), None),
    )
    .await
    .unwrap();
    let stage_before = read_opp(&pool, w.opportunity).await.stage_id;

    backbone_orm::company_scope::with_company_scope(
        Some(w.company),
        svc.lose_opportunity(w.opportunity, Some("price".into()), Some("acme".into()), &sink),
    )
    .await
    .unwrap();

    let row = read_opp(&pool, w.opportunity).await;
    assert_eq!(row.status, "lost");
    assert!(!row.active, "lost is archived");
    assert_eq!(row.probability, Decimal::ZERO);
    assert_eq!(row.stage_id, stage_before, "the row remembers where it died");
    assert!(row.date_closed.is_some());
    assert_eq!(sink.0.lock().unwrap().as_slice(), ["lost"]);
    let (reason, competitor): (Option<String>, Option<String>) =
        sqlx::query_as("SELECT lost_reason, competitor FROM deal.opportunities WHERE id=$1")
            .bind(w.opportunity)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(reason.as_deref(), Some("price"));
    assert_eq!(competitor.as_deref(), Some("acme"));

    // Terminal: a second lose and any stage move are both rejected.
    let err = backbone_orm::company_scope::with_company_scope(
        Some(w.company),
        svc.lose_opportunity(w.opportunity, None, None, &sink),
    )
    .await
    .unwrap_err();
    assert!(matches!(err, DealError::InvalidState(_)));
    assert_eq!(err.http_status(), 422);
    let err = backbone_orm::company_scope::with_company_scope(
        Some(w.company),
        svc.advance_stage(w.opportunity, stage_id_for(w.company, "closing"), None),
    )
    .await
    .unwrap_err();
    assert!(matches!(err, DealError::InvalidState(_)));
}

/// G5: a won→won slide never rewrites date_closed (it stamps only on first close), while the
/// stage-change stamp does move.
#[tokio::test]
async fn golden_won_to_won_preserves_date_closed() {
    let pool = pool().await;
    let company = Uuid::new_v4();
    seed_default_stages_sql(&pool, company).await;
    // A second won stage, sequenced after the seeded one.
    let won2 = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO deal.stages (id, company_id, code, name, sequence, is_won, probability_hint, active, metadata)
           VALUES ($1, $2, 'won-repeat', 'Won again', 70, TRUE, 100, TRUE, '{}'::jsonb)"#,
    )
    .bind(won2)
    .bind(company)
    .execute(&pool)
    .await
    .unwrap();

    let opportunity = new_opportunity(&pool, company, "Won-won slide", d("60")).await;
    add_line(&pool, company, opportunity).await;
    let svc = DealWriteService::new(pool.clone());
    let selling = FakeSelling { quotation: Uuid::new_v4(), calls: std::sync::Mutex::new(0) };
    let sink = RecordingSink::default();

    backbone_orm::company_scope::with_company_scope(
        Some(company),
        svc.win_opportunity(opportunity, &selling, &sink),
    )
    .await
    .unwrap();
    let after_win = read_opp(&pool, opportunity).await;
    assert_eq!(after_win.status, "won");
    assert_eq!(after_win.stage_id, stage_id_for(company, "won"), "win lands on the lowest-sequence won stage");

    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    backbone_orm::company_scope::with_company_scope(
        Some(company),
        svc.advance_stage(opportunity, won2, None),
    )
    .await
    .unwrap();
    let after_slide = read_opp(&pool, opportunity).await;
    assert_eq!(after_slide.status, "won");
    assert_eq!(after_slide.probability, Decimal::from(100), "the second won stage also forces 100");
    assert_eq!(after_slide.date_closed, after_win.date_closed, "won→won never rewrites date_closed");
    assert!(after_slide.date_last_stage_update > after_win.date_last_stage_update,
        "the actual stage change still stamps date_last_stage_update");
    assert_eq!(after_slide.quotation_id, Some(selling.quotation));
}

/// G6: the win verb picks the LOWEST-sequence active won stage, hands off exactly once (the
/// replay returns `already=true` with the same quotation), and a company with no selectable won
/// stage is rejected.
#[tokio::test]
async fn golden_win_picks_lowest_won_sequence_and_is_idempotent() {
    let pool = pool().await;
    let company = Uuid::new_v4();
    seed_default_stages_sql(&pool, company).await;
    // A higher-sequence won stage the verb must NOT pick.
    sqlx::query(
        r#"INSERT INTO deal.stages (id, company_id, code, name, sequence, is_won, probability_hint, active, metadata)
           VALUES ($1, $2, 'won-later', 'Won later', 90, TRUE, 100, TRUE, '{}'::jsonb)"#,
    )
    .bind(Uuid::new_v4())
    .bind(company)
    .execute(&pool)
    .await
    .unwrap();

    let opportunity = new_opportunity(&pool, company, "Win pick", d("50")).await;
    // The win needs a party (seeded by new_opportunity) + at least one line.
    add_line(&pool, company, opportunity).await;

    let svc = DealWriteService::new(pool.clone());
    let selling = FakeSelling { quotation: Uuid::new_v4(), calls: std::sync::Mutex::new(0) };
    let sink = RecordingSink::default();

    let out = backbone_orm::company_scope::with_company_scope(
        Some(company),
        svc.win_opportunity(opportunity, &selling, &sink),
    )
    .await
    .unwrap();
    assert!(!out.already);
    let row = read_opp(&pool, opportunity).await;
    assert_eq!(row.status, "won");
    assert_eq!(row.stage_id, stage_id_for(company, "won"), "seq 60 beats seq 90");
    assert_eq!(row.probability, Decimal::from(100));
    assert_eq!(row.quotation_id, Some(selling.quotation));
    assert_eq!(*selling.calls.lock().unwrap(), 1);
    assert_eq!(sink.0.lock().unwrap().as_slice(), ["won"]);

    // Replay: idempotent — same quotation, no second handoff, no second event.
    let replay = backbone_orm::company_scope::with_company_scope(
        Some(company),
        svc.win_opportunity(opportunity, &selling, &sink),
    )
    .await
    .unwrap();
    assert!(replay.already);
    assert_eq!(replay.quotation_id, selling.quotation);
    assert_eq!(*selling.calls.lock().unwrap(), 1);
    assert_eq!(sink.0.lock().unwrap().len(), 1);

    // A company whose only won stage is unselectable cannot win.
    let bare = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO deal.stages (id, company_id, code, name, sequence, is_won, probability_hint, active, metadata)
           VALUES ($1, $2, 'won-dead', 'Won (retired)', 60, TRUE, 100, FALSE, '{}'::jsonb)"#,
    )
    .bind(Uuid::new_v4())
    .bind(bare)
    .execute(&pool)
    .await
    .unwrap();
    let stuck = new_opportunity(&pool, bare, "No won stage", d("10")).await;
    let err = backbone_orm::company_scope::with_company_scope(
        Some(bare),
        svc.win_opportunity(stuck, &selling, &sink),
    )
    .await
    .unwrap_err();
    assert!(matches!(err, DealError::InvalidState(_)), "no active won stage: {err:?}");
    assert_eq!(err.http_status(), 422);
}

/// G7: the ADR-0015 DB backstops — a raw writer cannot hand-set status='won' on an inconsistent
/// row, nor write probability > 100. Status is reachable only through the gated statements.
#[tokio::test]
async fn golden_status_not_hand_settable() {
    let pool = pool().await;
    let w = seed_world(&pool, d("30")).await;

    let won_violation = sqlx::query("UPDATE deal.opportunities SET status='won'::opportunity_status WHERE id=$1")
        .bind(w.opportunity)
        .execute(&pool)
        .await
        .unwrap_err();
    assert!(
        won_violation.to_string().contains("opportunities_won_shape"),
        "hand-set won must hit the shape CHECK: {won_violation}"
    );

    let range_violation = sqlx::query("UPDATE deal.opportunities SET probability=101 WHERE id=$1")
        .bind(w.opportunity)
        .execute(&pool)
        .await
        .unwrap_err();
    assert!(
        range_violation.to_string().contains("opportunities_probability_range"),
        "probability 101 must hit the range CHECK: {range_violation}"
    );

    // And the lose shape: lost without archiving is equally unreachable for a raw writer.
    let lost_violation = sqlx::query("UPDATE deal.opportunities SET status='lost'::opportunity_status WHERE id=$1")
        .bind(w.opportunity)
        .execute(&pool)
        .await
        .unwrap_err();
    assert!(
        lost_violation.to_string().contains("opportunities_lost_shape"),
        "hand-set lost must hit the shape CHECK: {lost_violation}"
    );
}

/// G8: seed convergence — running the lazy ensure over a company the SQL side already seeded
/// inserts nothing and computes identical ids (the migration's uuid_generate_v5 and the Rust
/// new_v5 are the same function of (company, code)).
#[tokio::test]
async fn golden_seed_converges() {
    let pool = pool().await;
    let company = Uuid::new_v4();
    seed_default_stages_sql(&pool, company).await; // the SQL side (migration expression)

    let repo = OpportunityRepository::new(pool.clone());
    let mut conn = pool.acquire().await.unwrap();
    repo.ensure_default_stages(&mut conn, company).await.unwrap(); // the Rust side
    drop(conn);

    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM deal.stages WHERE company_id=$1")
        .bind(company)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 6, "the ensure over an already-seeded company adds nothing");

    for (code, _, _, _, _) in DEFAULT_STAGES {
        let id: Uuid = sqlx::query_scalar("SELECT id FROM deal.stages WHERE company_id=$1 AND code=$2")
            .bind(company)
            .bind(code)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(id, stage_id_for(company, code), "ids converge on the uuid5 of (company, code)");
    }
}

/// G9: lazy defaults — a company created after the migration gets its default stage set at first
/// opportunity insert, and the deal enters at the qualification stage.
#[tokio::test]
async fn golden_lazy_defaults_on_first_opportunity() {
    let pool = pool().await;
    let company = Uuid::new_v4();
    let n_before: i64 = sqlx::query_scalar("SELECT count(*) FROM deal.stages WHERE company_id=$1")
        .bind(company)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n_before, 0, "fresh company starts with no stages");

    let repo = OpportunityRepository::new(pool.clone());
    let id = Uuid::new_v4();
    let mut conn = pool.acquire().await.unwrap();
    repo.insert_opportunity(
        &mut conn,
        &NewOpportunityRow {
            id,
            company_id: company,
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

    let n_after: i64 = sqlx::query_scalar("SELECT count(*) FROM deal.stages WHERE company_id=$1")
        .bind(company)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n_after, 6, "the insert lazily seeded the default set");

    let row = read_opp(&pool, id).await;
    assert_eq!(row.stage_id, stage_id_for(company, "qualification"), "new deals enter at qualification");
    assert_eq!(row.status, "open");
    assert_eq!(row.probability, Decimal::ZERO);
    assert!(row.active);
    assert!(row.date_last_stage_update.is_some(), "the entry stamp is set at insert");
}

/// G10: the move gates + fences — a lost deal cannot move, an unknown stage id is a 404-shaped
/// stage_not_found, an inactive stage is a 422, and the STAGE fence itself: under a plain
/// non-superuser role bound to company B, company A's stage is invisible (the verb's probe sees
/// nothing — the cross-tenant shape), while company B sees its own.
#[tokio::test]
async fn golden_advance_gates_and_fences() {
    let pool = pool().await;
    let a = seed_world(&pool, d("20")).await;
    let b = seed_world(&pool, d("20")).await;
    let svc = DealWriteService::new(pool.clone());

    // Unknown stage id: 404-shaped named error.
    let err = backbone_orm::company_scope::with_company_scope(
        Some(a.company),
        svc.advance_stage(a.opportunity, Uuid::new_v4(), None),
    )
    .await
    .unwrap_err();
    assert!(matches!(err, DealError::StageNotFound(_)), "{err:?}");
    assert_eq!(err.http_status(), 404);

    // Inactive stage target: 422.
    sqlx::query(
        "INSERT INTO deal.stages (id, company_id, code, name, sequence, is_won, probability_hint, active, metadata)
         VALUES ($1, $2, 'frozen', 'Frozen', 55, FALSE, 0, FALSE, '{}'::jsonb)",
    )
    .bind(Uuid::new_v4())
    .bind(a.company)
    .execute(&pool)
    .await
    .unwrap();
    let frozen: Uuid = sqlx::query_scalar("SELECT id FROM deal.stages WHERE company_id=$1 AND code='frozen'")
        .bind(a.company)
        .fetch_one(&pool)
        .await
        .unwrap();
    let err = backbone_orm::company_scope::with_company_scope(
        Some(a.company),
        svc.advance_stage(a.opportunity, frozen, None),
    )
    .await
    .unwrap_err();
    assert!(matches!(err, DealError::StageInactive(_)), "{err:?}");
    assert_eq!(err.http_status(), 422);

    // Lost is terminal (G4 covers the full lose golden; here the gate leg).
    backbone_orm::company_scope::with_company_scope(
        Some(b.company),
        svc.lose_opportunity(b.opportunity, None, None, &RecordingSink::default()),
    )
    .await
    .unwrap();
    let err = backbone_orm::company_scope::with_company_scope(
        Some(b.company),
        svc.advance_stage(b.opportunity, stage_id_for(b.company, "closing"), None),
    )
    .await
    .unwrap_err();
    assert!(matches!(err, DealError::InvalidState(_)));

    // ── The fence leg: this suite connects as the DB owner (a superuser, whom RLS can never
    // bind even under FORCE). Run the probe the way production does — SET ROLE to a plain role
    // on one connection — and pin exactly the visibility the verb's stage probe rides.
    let mut conn = pool.acquire().await.unwrap();
    sqlx::query(
        r#"DO $$ BEGIN
               IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'deal_probe_rls') THEN
                   CREATE ROLE deal_probe_rls NOLOGIN;
               END IF;
           END $$"#,
    )
    .execute(&mut *conn)
    .await
    .unwrap();
    sqlx::query("GRANT USAGE ON SCHEMA deal TO deal_probe_rls").execute(&mut *conn).await.unwrap();
    sqlx::query("GRANT SELECT ON ALL TABLES IN SCHEMA deal TO deal_probe_rls")
        .execute(&mut *conn)
        .await
        .unwrap();
    sqlx::query("SET ROLE deal_probe_rls").execute(&mut *conn).await.unwrap();

    // Unbound (no tenant): zero rows by design — the fence default.
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM deal.stages")
        .fetch_one(&mut *conn)
        .await
        .unwrap();
    assert_eq!(n, 0, "unbound non-superuser sees zero stages");

    // Bound to company B: company A's stage — the cross-tenant move target — is invisible to
    // the exact probe query the move verb runs; B's own stage is visible.
    let a_stage = stage_id_for(a.company, "closing");
    let b_stage = stage_id_for(b.company, "closing");
    let mut tx = conn.begin().await.unwrap();
    sqlx::query("SELECT set_config('app.company_id', $1, true)")
        .bind(b.company.to_string())
        .execute(&mut *tx)
        .await
        .unwrap();
    let cross: Option<bool> = sqlx::query_scalar("SELECT active FROM deal.stages WHERE id=$1")
        .bind(a_stage)
        .fetch_optional(&mut *tx)
        .await
        .unwrap();
    assert!(cross.is_none(), "company B cannot see company A's stage (the probe returns nothing → 404 shape)");
    let own: bool = sqlx::query_scalar("SELECT active FROM deal.stages WHERE id=$1")
        .bind(b_stage)
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    assert!(own, "company B sees its own stage");
    tx.rollback().await.unwrap();

    sqlx::query("RESET ROLE").execute(&mut *conn).await.unwrap();
}
