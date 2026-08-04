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
/// gate (`status` + `quotation_id`).
pub struct OpportunityForWinRow {
    pub company_id: Uuid,
    pub party_id: Option<Uuid>,
    pub campaign_id: Option<Uuid>,
    pub currency: String,
    pub expected_amount: Decimal,
    pub status: String,
    pub quotation_id: Option<Uuid>,
}

/// Hand-written Opportunity SQL. Lives here (not in the write service) per the module's 4-layer rule.
impl OpportunityRepository {
    /// Insert the opportunity header. Takes the CALLER'S connection so the header, its lines, and the
    /// lead's status advance commit as one unit. The caller has already bound the company on that
    /// connection — don't re-bind here.
    pub async fn insert_opportunity(
        &self,
        conn: &mut sqlx::PgConnection,
        o: &NewOpportunityRow<'_>,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            r#"INSERT INTO deal.opportunities
                 (id, company_id, opportunity_name, lead_id, party_id, campaign_id, currency,
                  expected_amount, sales_stage, probability, expected_close_date, status)
               VALUES ($1,$2,$3,$4,$5,$6,$7,$8,'qualification'::sales_stage,0,$9,'open'::opportunity_status)"#,
        )
        .bind(o.id).bind(o.company_id).bind(o.opportunity_name).bind(o.lead_id).bind(o.party_id)
        .bind(o.campaign_id).bind(o.currency).bind(o.expected_amount).bind(o.expected_close_date)
        .execute(conn)
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

    /// Move an open deal's stage / probability. `stage` is bound as a free string and cast at the DB
    /// (`$2::sales_stage`). Returns rows affected (0 = the opportunity is not open).
    ///
    /// ID-only: no company argument. Runs `execute_scoped`, so it rides a connection carrying the
    /// caller's `app.company_id` and another company's deal simply is not updated. A non-request caller
    /// (event/job) must wrap this in `with_company_scope(Some(company_id))` or it fails closed.
    pub async fn advance_stage(
        &self,
        pool: &PgPool,
        opportunity_id: Uuid,
        stage: &str,
        probability: Decimal,
    ) -> Result<u64, sqlx::Error> {
        let moved = company_scope::execute_scoped(
            pool,
            sqlx::query(
                r#"UPDATE deal.opportunities SET sales_stage=$2::sales_stage, probability=$3
                   WHERE id=$1 AND status='open'::opportunity_status AND (metadata->>'deleted_at') IS NULL"#,
            )
            .bind(opportunity_id)
            .bind(stage)
            .bind(probability),
        )
        .await?;
        Ok(moved.rows_affected())
    }

    /// Read what the win decision needs, including the once-only gate.
    ///
    /// ID-only read — `fetch_optional_row_scoped` fences it to the caller's `app.company_id`. The
    /// company on the returned row is what the caller re-binds for the reads/writes that follow.
    pub async fn find_for_win(&self, pool: &PgPool, opportunity_id: Uuid) -> Result<Option<OpportunityForWinRow>, sqlx::Error> {
        let row = company_scope::fetch_optional_row_scoped(
            pool,
            sqlx::query(
                r#"SELECT company_id, party_id, campaign_id, currency, expected_amount, status::text AS status, quotation_id
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
            quotation_id: r.get("quotation_id"),
        }))
    }

    /// Claim the win exactly once (gated `open → won`). Returns rows affected (0 = a concurrent win
    /// took it; the caller re-reads the winner's quotation).
    ///
    /// Takes the pool and runs `execute_scoped`; the caller wraps this in
    /// `with_company_scope(Some(company_id))` using the company read off the deal.
    pub async fn claim_win(
        &self,
        pool: &PgPool,
        opportunity_id: Uuid,
        quotation_id: Uuid,
    ) -> Result<u64, sqlx::Error> {
        let moved = company_scope::execute_scoped(
            pool,
            sqlx::query(
                r#"UPDATE deal.opportunities
                   SET status='won'::opportunity_status, quotation_id=$2, probability=100
                   WHERE id=$1 AND status='open'::opportunity_status"#,
            )
            .bind(opportunity_id)
            .bind(quotation_id),
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

    /// Lose the deal (terminal), returning the company off the transitioned row so the caller can emit
    /// the event. `Ok(None)` = the opportunity is not open.
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
                   SET status='lost'::opportunity_status, lost_reason=$2, competitor=$3, probability=0
                   WHERE id=$1 AND status='open'::opportunity_status AND (metadata->>'deleted_at') IS NULL
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
