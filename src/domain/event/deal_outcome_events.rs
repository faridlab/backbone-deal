//! Deal outcome domain events (hand-authored, user-owned) — the public extension surface.
//!
//! Distinct from the generated CRUD-lifecycle `OpportunityEvent`: these are the terminal funnel
//! signals the write path publishes. `OpportunityWon` (the deal handed off to selling, which created
//! the Quotation/SO) and `OpportunityLost` (a read-side win/loss signal). Ported from backbone-crm's
//! `crm_events.rs` (OpportunityWon/Lost) + its `CrmEventSink` (now `DealEventSink`).

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A deal was WON — handed off to selling, which created the Quotation/SO.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OpportunityWon {
    pub opportunity_id: Uuid,
    pub party_id: Uuid,
    pub quotation_id: Uuid,
    pub company_id: Uuid,
    pub amount: Decimal,
    /// The campaign the winning deal is attributed to (snapshotted from the lead at qualify).
    /// This is what lets won revenue roll up by campaign — the KEEP attribution promise.
    pub campaign_id: Option<Uuid>,
}

/// A deal was LOST — a read-side win/loss signal.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OpportunityLost {
    pub opportunity_id: Uuid,
    pub company_id: Uuid,
    pub lost_reason: Option<String>,
    pub competitor: Option<String>,
}

/// The deal domain-event union.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type")]
pub enum DealEvent {
    OpportunityWon(OpportunityWon),
    OpportunityLost(OpportunityLost),
}

/// Sink the write path publishes to. A consuming service (the app) supplies its own (bus, outbox, …).
pub trait DealEventSink: Send + Sync {
    fn publish(&self, event: &DealEvent);
}

/// A no-op/logging sink for tests and single-process composition.
#[derive(Debug, Default, Clone)]
pub struct LoggingDealSink;

impl DealEventSink for LoggingDealSink {
    fn publish(&self, event: &DealEvent) {
        tracing::info!(?event, "deal event");
    }
}
