// This directory's extension (hand-written; ADR-0031). The generated mod.rs (lib.rs for
// src/) pulls it in with include!, so its lines resolve against this directory.

pub use opportunity_repository::{
    DEFAULT_STAGES, NewOpportunityRow, OpportunityForWinRow, StageMoveRow, StageProbeRow,
};
pub use opportunity_item_repository::{NewOppItemRow, OppItemLineRow};
