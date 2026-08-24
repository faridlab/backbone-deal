//! Repository for Opportunity entities
//!
//! Generated skeleton, now **user-owned** — this exact path is declared under `user_owned` in
//! `metaphor.codegen.yaml`, so the generator skips it wholesale. The custom methods below hold the
//! hand-written Opportunity SQL — the deal's stage moves and its once-only win/lose transitions
//! (4-layer rule: services orchestrate, repositories hold the SQL). Ported from backbone-crm with
//! one adaptation for the split: table `crm.opportunities` → `deal.opportunities`.
//!
//! Thin newtype over `backbone_orm::GenericCrudRepository<Opportunity, backbone_orm::SoftDelete>`.
//! All standard CRUD methods are available via `Deref`.

use anyhow::Result;
use rust_decimal::Decimal;
use sqlx::{PgPool, Row};
use uuid::Uuid;

use backbone_orm::company_scope;

use crate::domain::entity::Opportunity;

/// Table name for Opportunity entities
pub const TABLE_NAME: &str = "deal.opportunities";

/// The default pipeline every company starts with: the retiring sales_stage enum's five values
/// plus the seeded won stage. The entry probability numbers are advisory hints carried over from
/// the enum's implied funnel — a stage move never auto-applies them.
///
/// MUST stay in lockstep with the seeding VALUES list in
/// `migrations/20260821130002_stage_ref_reshape.up.sql`.
pub const DEFAULT_STAGES: [(&str, &str, i32, bool, i32); 6] = [
    ("prospecting", "Prospecting", 10, false, 10),
    ("qualification", "Qualification", 20, false, 20),
    ("proposal", "Proposal", 30, false, 40),
    ("negotiation", "Negotiation", 40, false, 70),
    ("closing", "Closing", 50, false, 90),
    ("won", "Won", 60, true, 100),
];

/// Deterministic id for one company's default stage — stable across re-seeds.
///
/// MUST stay byte-identical with the migration's
/// `uuid_generate_v5(uuid_ns_url(), 'deal-stage:' || company_id || ':' || code)` (same RFC-4122
/// URL namespace, same lowercase-hyphenated uuid text format): convergence between the seeded and
/// the lazily-ensured rows depends on it.
fn default_stage_id(company_id: Uuid, code: &str) -> Uuid {
    Uuid::new_v5(
        &Uuid::NAMESPACE_URL,
        format!("deal-stage:{company_id}:{code}").as_bytes(),
    )
}

/// Repository for Opportunity entities.
///
/// All standard CRUD, soft-delete, pagination, and bulk methods are
/// provided automatically via `Deref` to `backbone_orm::GenericCrudRepository`.
pub struct OpportunityRepository(
    backbone_orm::GenericCrudRepository<Opportunity, backbone_orm::SoftDelete>,
);

impl std::ops::Deref for OpportunityRepository {
    type Target = backbone_orm::GenericCrudRepository<Opportunity, backbone_orm::SoftDelete>;
    fn deref(&self) -> &Self::Target { &self.0 }
}

impl OpportunityRepository {
    /// Create a new repository instance.
    pub fn new(pool: PgPool) -> Self {
        Self(backbone_orm::GenericCrudRepository::new(pool, TABLE_NAME))
    }
}

/// The exact row a qualified opportunity writes.
///
/// Mirrors the raw column shape rather than the `Opportunity` entity. `expected_amount` is the caller's
/// already-rounded line total (IDR, 2dp, half-away-from-zero) — the money policy stays in the service.
/// The pipeline stage is NOT part of this shape: the repository resolves the entry stage
/// (the seeded `qualification` default) itself, so callers never see stage internals.
pub struct NewOpportunityRow<'a> {
    pub id: Uuid,
    pub company_id: Uuid,
    pub opportunity_name: &'a str,
    pub lead_id: Uuid,
    pub party_id: Option<Uuid>,
    pub campaign_id: Option<Uuid>,
    pub currency: &'a str,
    pub expected_amount: Decimal,
    pub expected_close_date: Option<chrono::DateTime<chrono::Utc>>,
}

/// The win pre-flight projection: the deal's company/party/attribution, its value, and the once-only
/// gate (`status` + `quotation_id` + the archive mark the claim gate rides on).
pub struct OpportunityForWinRow {
    pub company_id: Uuid,
    pub party_id: Option<Uuid>,
    pub campaign_id: Option<Uuid>,
    pub currency: String,
    pub expected_amount: Decimal,
    pub status: String,
    pub active: bool,
    pub quotation_id: Option<Uuid>,
}

/// What a gated stage move landed on.
pub struct StageMoveRow {
    pub status: String,
    pub probability: Decimal,
    pub date_closed: Option<chrono::DateTime<chrono::Utc>>,
}

/// The stage probe a move pre-check reads: does the target stage exist in the caller's tenant,
/// and is it selectable.
pub struct StageProbeRow {
    pub active: bool,
}

/// Hand-written Opportunity SQL. Lives here (not in the write service) per the module's 4-layer rule.
impl OpportunityRepository {
    /// Ensure the company owns its default stage set — the lazy half of the seeding contract
    /// (companies created after the reshape migration get their stages at first opportunity).
    /// Idempotent by construction: the ids are deterministic uuid5 values and
    /// `(company_id, code)` is unique, so `ON CONFLICT DO NOTHING` converges to the same rows the
    /// migration seeded.
    ///
    /// Takes the CALLER'S connection so this commits atomically with the opportunity insert that
    /// depends on it. The caller has already bound the company on that connection — don't re-bind.
    pub async fn ensure_default_stages(
        &self,
        conn: &mut sqlx::PgConnection,
        company_id: Uuid,
    ) -> Result<(), sqlx::Error> {
        for (code, name, sequence, is_won, hint) in DEFAULT_STAGES {
            sqlx::query(
                r#"INSERT INTO deal.stages
                       (id, company_id, code, name, sequence, is_won, probability_hint, active, metadata)
                   VALUES ($1,$2,$3,$4,$5,$6,$7,TRUE,
                           '{"created_at":null,"updated_at":null,"deleted_at":null,"created_by":null,"updated_by":null,"deleted_by":null}'::jsonb)
                   ON CONFLICT (company_id, code) DO NOTHING"#,
            )
            .bind(default_stage_id(company_id, code))
            .bind(company_id)
            .bind(code)
            .bind(name)
            .bind(sequence)
            .bind(is_won)
            .bind(Decimal::from(hint))
            .execute(&mut *conn)
            .await?;
        }
        Ok(())
    }

    /// Insert the opportunity header at the pipeline's entry stage (the seeded `qualification`
    /// default, resolved as its deterministic uuid). Takes the CALLER'S connection so the header,
    /// its lines, and the lead's status advance commit as one unit; the default stages are ensured
    /// on that same connection first. The caller has already bound the company on that
    /// connection — don't re-bind here.
    pub async fn insert_opportunity(
        &self,
        conn: &mut sqlx::PgConnection,
        o: &NewOpportunityRow<'_>,
    ) -> Result<(), sqlx::Error> {
        self.ensure_default_stages(&mut *conn, o.company_id).await?;
        sqlx::query(
            r#"INSERT INTO deal.opportunities
                 (id, company_id, opportunity_name, lead_id, party_id, campaign_id, currency,
                  expected_amount, stage_id, probability, expected_close_date, status, active,
                  date_last_stage_update)
               VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,0,$10,'open'::opportunity_status,TRUE,NOW())"#,
        )
        .bind(o.id).bind(o.company_id).bind(o.opportunity_name).bind(o.lead_id).bind(o.party_id)
        .bind(o.campaign_id).bind(o.currency).bind(o.expected_amount)
        .bind(default_stage_id(o.company_id, "qualification"))
        .bind(o.expected_close_date)
        .execute(&mut *conn)
        .await?;
        Ok(())
    }

    /// Back-fill the lead's open party-less opportunities with the newly minted party.
    ///
    /// An opportunity qualified BEFORE conversion snapshotted a NULL party; without this back-fill it
    /// would be permanently unwinnable even after the lead converts (council 2026-07-06). Opportunities
    /// qualified AFTER conversion already inherit the party at qualify time, so `party_id IS NULL`
    /// scopes the back-fill to exactly the stale ones.
    ///
    /// Takes the CALLER'S connection: this must commit atomically with the conversion claim it depends
    /// on. The caller has already bound the company on that connection — don't re-bind here.
    pub async fn backfill_party_for_lead(
        &self,
        conn: &mut sqlx::PgConnection,
        lead_id: Uuid,
        party_id: Uuid,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            r#"UPDATE deal.opportunities SET party_id=$2
               WHERE lead_id=$1 AND party_id IS NULL AND status='open'::opportunity_status"#,
        )
        .bind(lead_id)
        .bind(party_id)
        .execute(conn)
        .await?;
        Ok(())
    }

    /// Probe the move target: its row's `active` mark, or `None` when no such stage exists in the
    /// caller's tenant (an unknown and a cross-tenant stage id are indistinguishable by design —
    /// both read as absent).
    ///
    /// ID-only: no company argument. Runs `fetch_optional_row_scoped`, so the read rides a
    /// connection carrying the caller's `app.company_id` and another company's stage simply is not
    /// visible.
    pub async fn find_stage_for_move(&self, pool: &PgPool, stage_id: Uuid) -> Result<Option<StageProbeRow>, sqlx::Error> {
        let row = company_scope::fetch_optional_row_scoped(
            pool,
            sqlx::query("SELECT active FROM deal.stages WHERE id=$1").bind(stage_id),
        )
        .await?;
        Ok(row.map(|r| StageProbeRow { active: r.get("active") }))
    }

    /// Move a deal's stage, applying the derived-status rules in one gated statement:
    ///
    /// - entering an `is_won` stage FORCES probability to 100 (a body value cannot override it)
    ///   and derives `status='won'`;
    /// - an explicit probability edit applies; a plain move NEVER clobbers a manual probability
    ///     (the stage's `probability_hint` is advisory display data, never auto-applied);
    /// - `date_last_stage_update` stamps on every actual stage CHANGE (a same-stage probability
    ///   edit is not a stage change);
    /// - `date_closed` stamps on the FIRST close only: entering a won stage or reaching
    ///   probability >= 100 while unclosed — a won→won slide never rewrites it.
    ///
    /// The gate: only an open deal moves (a won deal may slide onto another `is_won` stage; lost is
    /// terminal), never an archived or soft-deleted row. `Ok(None)` = the opportunity did not move
    /// (not open / not found in tenant).
    ///
    /// ID-only: no company argument. Runs `fetch_optional_row_scoped`, so the update rides a
    /// connection carrying the caller's `app.company_id` and another company's deal simply is not
    /// updated. A non-request caller (event/job) must wrap this in
    /// `with_company_scope(Some(company_id))` or it fails closed.
    pub async fn advance_stage(
        &self,
        pool: &PgPool,
        opportunity_id: Uuid,
        to_stage_id: Uuid,
        probability: Option<Decimal>,
    ) -> Result<Option<StageMoveRow>, sqlx::Error> {
        let row = company_scope::fetch_optional_row_scoped(
            pool,
            sqlx::query(
                r#"UPDATE deal.opportunities o SET
                       stage_id = $2,
                       probability = CASE WHEN s.is_won THEN 100
                                          WHEN $3 IS NOT NULL THEN $3
                                          ELSE o.probability END,
                       status = CASE WHEN s.is_won THEN 'won'::opportunity_status
                                     ELSE o.status END,
                       date_last_stage_update = CASE WHEN o.stage_id IS DISTINCT FROM $2
                                                     THEN NOW() ELSE o.date_last_stage_update END,
                       date_closed = CASE WHEN (s.is_won OR COALESCE($3, o.probability) >= 100)
                                               AND o.date_closed IS NULL
                                          THEN NOW() ELSE o.date_closed END
                   FROM deal.stages s
                   WHERE s.id = $2 AND o.id = $1
                     AND (o.status = 'open'::opportunity_status
                          OR (o.status = 'won'::opportunity_status AND s.is_won))
                     AND o.active AND (o.metadata->>'deleted_at') IS NULL
                   RETURNING o.status::text AS status, o.probability, o.date_closed"#,
            )
            .bind(opportunity_id)
            .bind(to_stage_id)
            .bind(probability),
        )
        .await?;
        Ok(row.map(|r| StageMoveRow {
            status: r.get("status"),
            probability: r.get("probability"),
            date_closed: r.get("date_closed"),
        }))
    }

    /// Pick the stage the win verb lands on: the lowest-sequence active `is_won` stage the company
    /// configured. `Ok(None)` = the company has no selectable won stage. The caller wraps this in
    /// `with_company_scope(Some(company_id))` using the company read off the deal.
    pub async fn pick_won_stage(&self, pool: &PgPool, company_id: Uuid) -> Result<Option<Uuid>, sqlx::Error> {
        company_scope::fetch_optional_scalar_scoped(
            pool,
            sqlx::query_scalar(
                r#"SELECT id FROM deal.stages
                   WHERE company_id=$1 AND is_won AND active
                   ORDER BY sequence ASC LIMIT 1"#,
            )
            .bind(company_id),
        )
        .await
    }

    /// Read what the win decision needs, including the once-only gate.
    ///
    /// ID-only read — `fetch_optional_row_scoped` fences it to the caller's `app.company_id`. The
    /// company on the returned row is what the caller re-binds for the reads/writes that follow.
    pub async fn find_for_win(&self, pool: &PgPool, opportunity_id: Uuid) -> Result<Option<OpportunityForWinRow>, sqlx::Error> {
        let row = company_scope::fetch_optional_row_scoped(
            pool,
            sqlx::query(
                r#"SELECT company_id, party_id, campaign_id, currency, expected_amount,
                          status::text AS status, active, quotation_id
                   FROM deal.opportunities WHERE id=$1 AND (metadata->>'deleted_at') IS NULL"#,
            )
            .bind(opportunity_id),
        )
        .await?;
        Ok(row.map(|r| OpportunityForWinRow {
            company_id: r.get("company_id"),
            party_id: r.get("party_id"),
            campaign_id: r.get("campaign_id"),
            currency: r.get("currency"),
            expected_amount: r.get("expected_amount"),
            status: r.get("status"),
            active: r.get("active"),
            quotation_id: r.get("quotation_id"),
        }))
    }

    /// Claim the win exactly once (gated `open → won`, or the quotation-less drag-to-won completion:
    /// a deal already sitting on a won stage via a plain move can still hand off — `quotation_id`
    /// NULL-ness keeps "hands off at most once" true). Lands the deal on the given won stage with
    /// probability 100. Returns rows affected (0 = a concurrent win took it; the caller re-reads
    /// the winner's quotation).
    ///
    /// Takes the pool and runs `execute_scoped`; the caller wraps this in
    /// `with_company_scope(Some(company_id))` using the company read off the deal.
    pub async fn claim_win(
        &self,
        pool: &PgPool,
        opportunity_id: Uuid,
        quotation_id: Uuid,
        won_stage_id: Uuid,
    ) -> Result<u64, sqlx::Error> {
        let moved = company_scope::execute_scoped(
            pool,
            sqlx::query(
                r#"UPDATE deal.opportunities
                   SET status='won'::opportunity_status, quotation_id=$2, probability=100,
                       stage_id=$3, date_last_stage_update=NOW(),
                       date_closed=COALESCE(date_closed, NOW())
                   WHERE id=$1 AND active
                     AND (status='open'::opportunity_status
                          OR (status='won'::opportunity_status AND quotation_id IS NULL))"#,
            )
            .bind(opportunity_id)
            .bind(quotation_id)
            .bind(won_stage_id),
        )
        .await?;
        Ok(moved.rows_affected())
    }

    /// Re-read the winner's quotation id after a losing win CAS. The caller wraps this in
    /// `with_company_scope(Some(company_id))`.
    pub async fn fetch_quotation_id(&self, pool: &PgPool, opportunity_id: Uuid) -> Result<Uuid, sqlx::Error> {
        company_scope::fetch_one_scalar_scoped(
            pool,
            sqlx::query_scalar("SELECT quotation_id FROM deal.opportunities WHERE id=$1").bind(opportunity_id),
        )
        .await
    }

    /// Lose the deal (terminal): derives `status='lost'` by archiving — probability 0 + the archive
    /// mark — stamping `date_closed` on first close, keeping the last pipeline stage (the row
    /// remembers where it died; `date_last_stage_update` is NOT stamped, no stage changed). Returns
    /// the company off the transitioned row so the caller can emit the event. `Ok(None)` = the
    /// opportunity is not open.
    ///
    /// ID-only: no company argument. The gated UPDATE..RETURNING rides the request-dedicated connection
    /// via `fetch_optional_scalar_scoped`, so RLS fences it to the caller's tenant. A non-request caller
    /// must wrap this in `with_company_scope(Some(company_id))` or it fails closed.
    pub async fn lose(
        &self,
        pool: &PgPool,
        opportunity_id: Uuid,
        lost_reason: Option<&str>,
        competitor: Option<&str>,
    ) -> Result<Option<Uuid>, sqlx::Error> {
        company_scope::fetch_optional_scalar_scoped(
            pool,
            sqlx::query_scalar(
                r#"UPDATE deal.opportunities
                   SET status='lost'::opportunity_status, lost_reason=$2, competitor=$3,
                       probability=0, active=FALSE, date_closed=COALESCE(date_closed, NOW())
                   WHERE id=$1 AND status='open'::opportunity_status AND active
                     AND (metadata->>'deleted_at') IS NULL
                   RETURNING company_id"#,
            )
            .bind(opportunity_id)
            .bind(lost_reason)
            .bind(competitor),
        )
        .await
    }
}

backbone_core::impl_crud_repository!(OpportunityRepository, Opportunity, soft_delete);
