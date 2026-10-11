// This directory's extension (hand-written; ADR-0031). The generated mod.rs (lib.rs for
// src/) pulls it in with include!, so its lines resolve against this directory.

pub mod deal_ports;
pub mod deal_write_service;
pub use deal_ports::{SellingPort, QuotationFromOpp, OppLine, QuotationAck, CrmRejected};
pub use deal_write_service::{DealWriteService, WinOutcome, DealError};
