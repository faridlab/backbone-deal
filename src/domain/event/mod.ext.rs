// This directory's extension (hand-written; ADR-0031). The generated mod.rs (lib.rs for
// src/) pulls it in with include!, so its lines resolve against this directory.

pub mod deal_outcome_events;
pub use deal_outcome_events::{
    OpportunityWon, OpportunityLost, DealEvent, DealEventSink, LoggingDealSink,
};
