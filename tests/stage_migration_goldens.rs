//! Migration oracle for the stage_ref reshape — the legacy-data backfill and the fresh-database
//! chain, proven against real Postgres.
//!
//! Each golden builds its own throwaway database from the repo's `migrations/` directory (read at
//! run time, applied with the multi-statement runner): M1 stops before the two stage migrations,
//! inserts legacy `sales_stage`-shaped rows (including a hand-corrupted won probability), then
//! applies the held-back pair in order and pins the mapping; M2 applies the FULL chain on an empty
//! database (no organization schema) and pins the fresh path.
//!
//! Scratch databases are created inside the shared dev container and dropped when the golden
//! finishes (and before it starts, so re-runs converge).

use sqlx::{Executor, PgPool};
use uuid::Uuid;

const MIGRATIONS_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/migrations");

/// The two migrations under test, identified by filename prefix in sort order.
const STAGE_TABLE_MIGRATION: &str = "20260821130001_create_stage_table";
const STAGE_RESHAPE_MIGRATION: &str = "20260821130002_stage_ref_reshape";

fn admin_url() -> String {
    // DATABASE_URL points at the module's scratch DB; the admin connection swaps the path for
    // the container's maintenance database (createdb/dropdb need it).
    let url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgresql://serpa:serpa_dev_password@127.0.0.1:5432/deal_stage_goldens".into());
    let (prefix, _) = url.rsplit_once('/').expect("DATABASE_URL with a database path");
    format!("{prefix}/serpa")
}

async fn admin_pool() -> PgPool {
    PgPool::connect(&admin_url()).await.expect("connect admin DB")
}

async fn recreate(admin: &PgPool, name: &str) -> PgPool {
    let drop = format!(r#"DROP DATABASE IF EXISTS {name}"#);
    let create = format!(r#"CREATE DATABASE {name}"#);
    admin.execute(drop.as_str()).await.expect("drop scratch DB");
    admin.execute(create.as_str()).await.expect("create scratch DB");
    let url = {
        let base = admin_url();
        let (prefix, _) = base.rsplit_once('/').unwrap();
        format!("{prefix}/{name}")
    };
    PgPool::connect(&url).await.expect("connect scratch DB")
}

/// Every `.up.sql` in the migrations directory, sorted (the chain order), as (prefix, sql).
fn migration_files() -> Vec<(String, String)> {
    let mut entries: Vec<(String, String)> = std::fs::read_dir(MIGRATIONS_DIR)
        .expect("read migrations dir")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "sql") && p.file_name().is_some_and(|n| n.to_string_lossy().ends_with(".up.sql")))
        .filter_map(|p| {
            let name = p.file_name()?.to_string_lossy().to_string();
            let body = std::fs::read_to_string(&p).ok()?;
            Some((name, body))
        })
        .collect();
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    entries
}

async fn apply_all(pool: &PgPool, skip: &[&str]) {
    for (name, sql) in migration_files() {
        if skip.iter().any(|s| name.starts_with(s)) {
            continue;
        }
        sqlx::raw_sql(&sql)
            .execute(pool)
            .await
            .unwrap_or_else(|e| panic!("applying {name}: {e}"));
    }
}

async fn apply_one(pool: &PgPool, prefix: &str) {
    let (name, sql) = migration_files()
        .into_iter()
        .find(|(n, _)| n.starts_with(prefix))
        .unwrap_or_else(|| panic!("migration {prefix} not found"));
    sqlx::raw_sql(&sql)
        .execute(pool)
        .await
        .unwrap_or_else(|e| panic!("applying {name}: {e}"));
}

fn stage_id_for(company: Uuid, code: &str) -> Uuid {
    Uuid::new_v5(&Uuid::NAMESPACE_URL, format!("deal-stage:{company}:{code}").as_bytes())
}

/// M1: legacy rows map onto the seeded stages exactly — open keeps its enum code's stage and
/// probability, won normalizes onto the is_won stage with probability 100 + date_closed (however
/// corrupted the legacy probability was), lost keeps its last pipeline stage archived at 0 — and
/// every id is the deterministic uuid5, the old column + type are gone, and the status-shaped
/// index exists.
#[tokio::test]
async fn golden_backfill_maps_enum_to_stages() {
    let admin = admin_pool().await;
    let pool = recreate(&admin, "deal_stage_migration_m1").await;

    // The pre-reshape chain: everything except the two stage migrations.
    apply_all(&pool, &[STAGE_TABLE_MIGRATION, STAGE_RESHAPE_MIGRATION]).await;

    let company = Uuid::new_v4();
    let mk = |id: Uuid, stage: &str, probability: &str, status: &str| {
        format!(
            r#"INSERT INTO deal.opportunities
                   (id, company_id, opportunity_name, lead_id, currency, expected_amount,
                    sales_stage, probability, status{extra_cols})
               VALUES ('{id}', '{company}', 'legacy', '{lead}', 'IDR', 100,
                       '{stage}'::sales_stage, {probability}, '{status}'::opportunity_status{extra_vals})"#,
            id = id,
            company = company,
            lead = Uuid::new_v4(),
            stage = stage,
            probability = probability,
            status = status,
            extra_cols = if status == "won" { ", quotation_id" } else { "" },
            extra_vals = if status == "won" {
                format!(", '{}'", Uuid::new_v4())
            } else {
                String::new()
            },
        )
    };
    let legacy_open = Uuid::new_v4();
    let legacy_won = Uuid::new_v4();
    let legacy_lost = Uuid::new_v4();
    sqlx::raw_sql(&mk(legacy_open, "negotiation", "70", "open")).execute(&pool).await.unwrap();
    // The won row's probability is hand-corrupted below 100 — the backfill must normalize it.
    sqlx::raw_sql(&mk(legacy_won, "closing", "40", "won")).execute(&pool).await.unwrap();
    sqlx::raw_sql(&mk(legacy_lost, "proposal", "10", "lost")).execute(&pool).await.unwrap();

    // The held-back pair, in order.
    apply_one(&pool, STAGE_TABLE_MIGRATION).await;
    apply_one(&pool, STAGE_RESHAPE_MIGRATION).await;

    // Open: its enum code's stage, manual probability kept, active, unclosed.
    let (stage, probability, active, closed): (Uuid, rust_decimal::Decimal, bool, bool) =
        sqlx::query_as(
            "SELECT stage_id, probability, active, date_closed IS NOT NULL \
             FROM deal.opportunities WHERE id=$1",
        )
        .bind(legacy_open)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(stage, stage_id_for(company, "negotiation"));
    assert_eq!(probability, rust_decimal::Decimal::from(70));
    assert!(active);
    assert!(!closed);

    // Won: the is_won stage, probability normalized to 100, closed, quotation kept.
    let (stage, probability, active, closed, has_quote): (Uuid, rust_decimal::Decimal, bool, bool, bool) =
        sqlx::query_as(
            "SELECT stage_id, probability, active, date_closed IS NOT NULL, quotation_id IS NOT NULL \
             FROM deal.opportunities WHERE id=$1",
        )
        .bind(legacy_won)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(stage, stage_id_for(company, "won"));
    assert_eq!(probability, rust_decimal::Decimal::from(100), "corrupted legacy won probability normalizes");
    assert!(active);
    assert!(closed);
    assert!(has_quote);

    // Lost: keeps its last pipeline stage, archived at 0, closed.
    let (stage, probability, active, closed): (Uuid, rust_decimal::Decimal, bool, bool) =
        sqlx::query_as(
            "SELECT stage_id, probability, active, date_closed IS NOT NULL \
             FROM deal.opportunities WHERE id=$1",
        )
        .bind(legacy_lost)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(stage, stage_id_for(company, "proposal"), "lost keeps where it died");
    assert_eq!(probability, rust_decimal::Decimal::ZERO);
    assert!(!active);
    assert!(closed);

    // Seeded set: six stages per legacy company, every id the deterministic uuid5.
    let seeded: Vec<(String, Uuid)> = sqlx::query_as(
        "SELECT code, id FROM deal.stages WHERE company_id=$1 ORDER BY sequence",
    )
    .bind(company)
    .fetch_all(&pool)
    .await
    .unwrap();
    let codes: Vec<&str> = seeded.iter().map(|(c, _)| c.as_str()).collect();
    assert_eq!(
        codes,
        ["prospecting", "qualification", "proposal", "negotiation", "closing", "won"]
    );
    for (code, id) in &seeded {
        assert_eq!(*id, stage_id_for(company, code));
    }

    // The retired column + type are gone; the status/stage index exists.
    let sales_stage_cols: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM information_schema.columns \
         WHERE table_schema='deal' AND table_name='opportunities' AND column_name='sales_stage'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(sales_stage_cols, 0, "the sales_stage column is gone");
    let sales_stage_type: i64 =
        sqlx::query_scalar("SELECT count(*) FROM pg_type WHERE typname='sales_stage'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(sales_stage_type, 0, "the sales_stage enum type is retired");
    let status_stage_idx: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pg_indexes \
         WHERE schemaname='deal' AND indexname='idx_opportunities_company_id_status_stage_id'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(status_stage_idx, 1);

    // Close the scratch pool's sessions before the drop (a live session blocks DROP DATABASE).
    pool.close().await;
    admin.execute("DROP DATABASE IF EXISTS deal_stage_migration_m1").await.unwrap();
}

/// M2: the full chain applies clean on an empty database (no organization schema — the seed's
/// organization branch is skipped), stages stay empty with no companies, and the strict fence is
/// armed on deal.stages.
#[tokio::test]
async fn golden_fresh_database_chain() {
    let admin = admin_pool().await;
    let pool = recreate(&admin, "deal_stage_migration_m2").await;

    apply_all(&pool, &[]).await; // nothing skipped — the whole chain, fresh DB

    let stages: i64 = sqlx::query_scalar("SELECT count(*) FROM deal.stages")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(stages, 0, "no companies exist — nothing was seeded");

    let fenced: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pg_policies \
         WHERE schemaname='deal' AND tablename='stages' AND policyname='stages_company_isolation'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(fenced, 1, "the strict company fence is armed on deal.stages");

    let forced: bool = sqlx::query_scalar(
        "SELECT relforcerowsecurity FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace \
         WHERE n.nspname='deal' AND c.relname='stages'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(forced, "RLS is forced on deal.stages");

    let checks: Vec<String> = sqlx::query_scalar(
        "SELECT conname FROM pg_constraint \
         WHERE conrelid = 'deal.opportunities'::regclass AND contype='c' ORDER BY conname",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    for expected in [
        "opportunities_probability_range",
        "opportunities_won_shape",
        "opportunities_lost_shape",
    ] {
        assert!(checks.iter().any(|c| c == expected), "missing CHECK {expected}: {checks:?}");
    }

    // Close the scratch pool's sessions before the drop (a live session blocks DROP DATABASE).
    pool.close().await;
    admin.execute("DROP DATABASE IF EXISTS deal_stage_migration_m2").await.unwrap();
}
