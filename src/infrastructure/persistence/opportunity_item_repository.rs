//! Repository for OpportunityItem entities
//!
//! Generated skeleton, now **user-owned** — declared under `user_owned` in `metaphor.codegen.yaml`.
//! The custom methods below hold the hand-written OpportunityItem SQL (4-layer rule: services
//! orchestrate, repos hold SQL). Ported from backbone-crm: table `crm.opportunity_items` →
//! `deal.opportunity_items`.
//!
//! Thin newtype over `backbone_orm::GenericCrudRepository<OpportunityItem, backbone_orm::SoftDelete>`.
//! All standard CRUD methods are available via `Deref`.

use anyhow::Result;
use rust_decimal::Decimal;
use sqlx::{PgPool, Row};
use uuid::Uuid;

use backbone_orm::company_scope;

use crate::domain::entity::OpportunityItem;

/// Table name for OpportunityItem entities
pub const TABLE_NAME: &str = "deal.opportunity_items";

/// Repository for OpportunityItem entities.
///
/// All standard CRUD, soft-delete, pagination, and bulk methods are
/// provided automatically via `Deref` to `backbone_orm::GenericCrudRepository`.
pub struct OpportunityItemRepository(
    backbone_orm::GenericCrudRepository<OpportunityItem, backbone_orm::SoftDelete>,
);

impl std::ops::Deref for OpportunityItemRepository {
    type Target = backbone_orm::GenericCrudRepository<OpportunityItem, backbone_orm::SoftDelete>;
    fn deref(&self) -> &Self::Target { &self.0 }
}

impl OpportunityItemRepository {
    /// Create a new repository instance.
    pub fn new(pool: PgPool) -> Self {
        Self(backbone_orm::GenericCrudRepository::new(pool, TABLE_NAME))
    }
}

/// The exact row an opportunity line writes.
///
/// Mirrors the raw column shape rather than the `OpportunityItem` entity. `amount` is the caller's
/// already-rounded extension (qty x rate, IDR 2dp half-away-from-zero) — the money policy stays in the
/// service, not in the SQL.
pub struct NewOppItemRow<'a> {
    pub id: Uuid,
    pub opportunity_id: Uuid,
    pub company_id: Uuid,
    pub item_id: Uuid,
    pub description: Option<&'a str>,
    pub quantity: Decimal,
    pub rate: Decimal,
    pub amount: Decimal,
}

/// The hand-off projection: what selling needs to quote each line.
pub struct OppItemLineRow {
    pub item_id: Uuid,
    pub quantity: Decimal,
    pub rate: Decimal,
}

/// Hand-written OpportunityItem SQL. Lives here (not in the write service) per the module's 4-layer rule.
impl OpportunityItemRepository {
    /// Insert one opportunity line. Takes the CALLER'S connection so it commits with the header it
    /// belongs to. The caller has already bound the company on that connection — don't re-bind here.
    pub async fn insert_item(&self, conn: &mut sqlx::PgConnection, l: &NewOppItemRow<'_>) -> Result<(), sqlx::Error> {
        sqlx::query(
            r#"INSERT INTO deal.opportunity_items
                 (id, opportunity_id, company_id, item_id, description, quantity, rate, amount)
               VALUES ($1,$2,$3,$4,$5,$6,$7,$8)"#,
        )
        .bind(l.id).bind(l.opportunity_id).bind(l.company_id).bind(l.item_id).bind(l.description).bind(l.quantity)
        .bind(l.rate).bind(l.amount)
        .execute(conn)
        .await?;
        Ok(())
    }

    /// The deal's lines, as selling needs them to build the quotation.
    ///
    /// A read outside any transaction: takes the pool and runs `fetch_all_rows_scoped` so the RLS fence
    /// (ADR-0008) applies. The caller wraps this in `with_company_scope(Some(company_id))` using the
    /// company read off the opportunity.
    pub async fn list_lines(&self, pool: &PgPool, opportunity_id: Uuid) -> Result<Vec<OppItemLineRow>, sqlx::Error> {
        let rows = company_scope::fetch_all_rows_scoped(
            pool,
            sqlx::query(
                r#"SELECT item_id, quantity, rate FROM deal.opportunity_items WHERE opportunity_id=$1"#,
            )
            .bind(opportunity_id),
        )
        .await?;
        Ok(rows
            .iter()
            .map(|r| OppItemLineRow { item_id: r.get("item_id"), quantity: r.get("quantity"), rate: r.get("rate") })
            .collect())
    }
}

backbone_core::impl_crud_repository!(OpportunityItemRepository, OpportunityItem, soft_delete);
