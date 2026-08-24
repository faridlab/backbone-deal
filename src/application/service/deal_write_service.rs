//! The hand-authored Deal write path (user-owned; survives regen).
//!
//! Stage moves + win/lose over the stage-driven lifecycle: won/lost is DERIVED
//! (stage.is_won + probability + the archive mark), never hand-set — the only
//! write paths are the gated statements in the repository below. The win hands
//! a deal off to selling via `SellingPort` (idempotent per opportunity); lose is
//! terminal. Posts NO GL (no money has moved yet). Money is IDR, 2dp,
//! half-away-from-zero.
//!
//! These flows touch only deal-owned tables + the SellingPort, so they live HERE (single-module).
//! The cross-module orchestration — `qualify_lead` and `convert_lead`, which span lead + deal repos
//! in one transaction — lives in backbone-crm-app. Ported from backbone-crm's `crm_write_service.rs`
//! (deal parts).

use backbone_orm::company_scope;
use rust_decimal::Decimal;
use sqlx::PgPool;
use uuid::Uuid;

use crate::domain::event::{DealEvent, DealEventSink, OpportunityLost, OpportunityWon};
use crate::infrastructure::persistence::{OppItemLineRow, OpportunityItemRepository, OpportunityRepository};

use super::deal_ports::{OppLine, QuotationFromOpp, SellingPort};

#[derive(Debug, thiserror::Error)]
pub enum DealError {
    #[error("db: {0}")]
    Db(#[from] sqlx::Error),
    #[error("not found: {0}")]
    NotFound(&'static str),
    #[error("stage not found: {0}")]
    StageNotFound(Uuid),
    #[error("invalid state: {0}")]
    InvalidState(&'static str),
    #[error("stage is inactive: {0}")]
    StageInactive(Uuid),
    #[error("invalid input: {0}")]
    Invalid(String),
    #[error("selling rejected: {0}")]
    SellingRejected(String),
}

impl DealError {
    /// Stable machine code for the HTTP error body (the guarded-route surface pins these).
    pub fn code(&self) -> String {
        match self {
            DealError::Db(_) => "internal_error".into(),
            DealError::NotFound(_) => "not_found".into(),
            DealError::StageNotFound(_) => "stage_not_found".into(),
            DealError::InvalidState(_) => "invalid_state".into(),
            DealError::StageInactive(_) => "stage_inactive".into(),
            DealError::Invalid(_) => "invalid_input".into(),
            DealError::SellingRejected(_) => "selling_rejected".into(),
        }
    }
    pub fn http_status(&self) -> u16 {
        match self {
            DealError::Db(_) => 500,
            DealError::NotFound(_) | DealError::StageNotFound(_) => 404,
            _ => 422,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct WinOutcome {
    pub quotation_id: Uuid,
    pub amount: Decimal,
    pub already: bool,
}

/// What a stage move landed on — the derived status, the effective probability,
/// and the (possibly just-stamped) close timestamp.
#[derive(Debug, Clone, PartialEq)]
pub struct StageMoveOutcome {
    pub status: String,
    pub probability: Decimal,
    pub date_closed: Option<chrono::DateTime<chrono::Utc>>,
}

pub struct DealWriteService {
    pool: PgPool,
    opportunities: OpportunityRepository,
    opportunity_items: OpportunityItemRepository,
}

impl DealWriteService {
    pub fn new(pool: PgPool) -> Self {
        let opportunities = OpportunityRepository::new(pool.clone());
        let opportunity_items = OpportunityItemRepository::new(pool.clone());
        Self { pool, opportunities, opportunity_items }
    }

    /// Move an opportunity's stage. Entering an `is_won` stage forces probability 100 and derives
    /// status=won; an explicit `probability` edit applies on any move (including a same-stage edit);
    /// a move with no probability value preserves the manual probability untouched.
    pub async fn advance_stage(
        &self,
        opportunity_id: Uuid,
        to_stage_id: Uuid,
        probability: Option<Decimal>,
    ) -> Result<StageMoveOutcome, DealError> {
        if let Some(p) = probability {
            if p < Decimal::ZERO || p > Decimal::from(100) {
                return Err(DealError::Invalid("probability must be 0..100".into()));
            }
        }
        // Two-probe error split (the family convention): a missing/inactive TARGET stage is a
        // named, actionable error; a target the caller's tenant cannot see reads as absent.
        // RLS scope (ADR-0008), ID-only pattern: no company argument — both probes ride the
        // request-dedicated connection, so RLS fences them to the caller's tenant.
        let stage = self
            .opportunities
            .find_stage_for_move(&self.pool, to_stage_id)
            .await?
            .ok_or(DealError::StageNotFound(to_stage_id))?;
        if !stage.active {
            return Err(DealError::StageInactive(to_stage_id));
        }
        let moved = self
            .opportunities
            .advance_stage(&self.pool, opportunity_id, to_stage_id, probability)
            .await?
            .ok_or(DealError::InvalidState("opportunity is not open"))?;
        Ok(StageMoveOutcome {
            status: moved.status,
            probability: moved.probability,
            date_closed: moved.date_closed,
        })
    }

    /// Win the deal — hand it off to selling as a Quotation/SO. Requires a party + at least one line.
    /// Drives `SellingPort::create_quotation` (idempotent per opportunity), then transition-gates
    /// the claim with the created `quotation_id`. Hands off **at most once** (a deal already sitting
    /// on a won stage without a quotation — a drag-to-won move — completes its handoff here).
    pub async fn win_opportunity(
        &self,
        opportunity_id: Uuid,
        selling: &dyn SellingPort,
        sink: &dyn DealEventSink,
    ) -> Result<WinOutcome, DealError> {
        // RLS scope (ADR-0008), ID-only pattern — the header read is fenced by the request-dedicated
        // connection; the reads/writes below re-bind the opportunity's own company.
        let opp = self
            .opportunities
            .find_for_win(&self.pool, opportunity_id)
            .await?
            .ok_or(DealError::NotFound("opportunity"))?;
        let amount: Decimal = opp.expected_amount;
        if opp.status == "won" {
            let q: Uuid = opp
                .quotation_id
                .ok_or(DealError::InvalidState("won without a quotation"))?;
            return Ok(WinOutcome { quotation_id: q, amount, already: true });
        }
        if opp.status != "open" || !opp.active {
            return Err(DealError::InvalidState("opportunity is not open"));
        }
        let party_id: Uuid = opp
            .party_id
            .ok_or(DealError::Invalid("opportunity has no party — convert the lead first".into()))?;
        let company_id = opp.company_id;
        let campaign_id: Option<Uuid> = opp.campaign_id;
        let currency: String = opp.currency;

        // The won stage this company lands on: its lowest-sequence active is_won stage.
        let won_stage_id = company_scope::with_company_scope(
            Some(company_id),
            self.opportunities.pick_won_stage(&self.pool, company_id),
        )
        .await?
        .ok_or(DealError::InvalidState("company has no active won stage"))?;

        let line_rows: Vec<OppItemLineRow> = company_scope::with_company_scope(
            Some(company_id),
            self.opportunity_items.list_lines(&self.pool, opportunity_id),
        )
        .await?;
        let lines: Vec<OppLine> = line_rows
            .iter()
            .map(|r| OppLine { item_id: r.item_id, quantity: r.quantity, rate: r.rate })
            .collect();
        if lines.is_empty() {
            return Err(DealError::Invalid("opportunity has no lines to quote".into()));
        }

        // Hand off to selling (idempotent per opportunity_id).
        let ack = selling
            .create_quotation(&QuotationFromOpp { company_id, opportunity_id, party_id, currency, lines })
            .await
            .map_err(|r| DealError::SellingRejected(r.code))?;

        // Gate: claim the win exactly once.
        let moved = company_scope::with_company_scope(
            Some(company_id),
            self.opportunities.claim_win(&self.pool, opportunity_id, ack.quotation_id, won_stage_id),
        )
        .await?;
        if moved != 1 {
            let q: Uuid = company_scope::with_company_scope(
                Some(company_id),
                self.opportunities.fetch_quotation_id(&self.pool, opportunity_id),
            )
            .await?;
            return Ok(WinOutcome { quotation_id: q, amount, already: true });
        }
        sink.publish(&DealEvent::OpportunityWon(OpportunityWon {
            opportunity_id,
            party_id,
            quotation_id: ack.quotation_id,
            company_id,
            amount,
            campaign_id,
        }));
        Ok(WinOutcome { quotation_id: ack.quotation_id, amount, already: false })
    }

    /// Lose the deal — terminal (archive + probability 0 derives status=lost; the stage is kept as
    /// the record of where the deal died). Emits `OpportunityLost`.
    pub async fn lose_opportunity(
        &self,
        opportunity_id: Uuid,
        lost_reason: Option<String>,
        competitor: Option<String>,
        sink: &dyn DealEventSink,
    ) -> Result<(), DealError> {
        // RLS scope (ADR-0008), ID-only pattern: the gated UPDATE..RETURNING rides the request-dedicated
        // connection, so RLS fences it to the caller's tenant.
        let company_id: Option<Uuid> = self
            .opportunities
            .lose(&self.pool, opportunity_id, lost_reason.as_deref(), competitor.as_deref())
            .await?;
        match company_id {
            Some(cid) => {
                sink.publish(&DealEvent::OpportunityLost(OpportunityLost {
                    opportunity_id,
                    company_id: cid,
                    lost_reason,
                    competitor,
                }));
                Ok(())
            }
            None => Err(DealError::InvalidState("opportunity is not open")),
        }
    }
}
