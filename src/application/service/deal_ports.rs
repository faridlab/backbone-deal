//! Deal's forward-conversion port — the exit where a won Opportunity hands off to selling.
//!
//! backbone-deal holds only this trait + its DTOs; a composing service (backbone-crm-app)
//! implements it over backbone-selling. **Zero normal Cargo edge** to backbone-selling — the DTOs
//! are the wire contract, duplicated per consumer by design. Ported from backbone-crm's
//! `crm_ports.rs` (SellingPort + QuotationFromOpp + OppLine + QuotationAck); the PartyPort stays
//! in backbone-lead.

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// One line carried from the opportunity into the Quotation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OppLine {
    pub item_id: Uuid,
    pub quantity: Decimal,
    pub rate: Decimal,
}

/// Hand a won opportunity off to selling as a Quotation/Sales Order.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct QuotationFromOpp {
    pub company_id: Uuid,
    pub opportunity_id: Uuid,
    pub party_id: Uuid,
    pub currency: String,
    pub lines: Vec<OppLine>,
}

/// The created Quotation/Sales Order id.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct QuotationAck {
    pub quotation_id: Uuid,
}

/// A downstream rejection surfaced to the deal module.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CrmRejected {
    pub code: String,
    pub message: String,
}

/// The selling seam — a composing service implements it over backbone-selling.
#[async_trait::async_trait]
pub trait SellingPort: Send + Sync {
    async fn create_quotation(&self, req: &QuotationFromOpp) -> Result<QuotationAck, CrmRejected>;
}
