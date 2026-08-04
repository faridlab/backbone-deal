use serde::{Deserialize, Serialize};
use sqlx::Type;
use std::str::FromStr;
#[cfg(feature = "openapi")]
use utoipa::ToSchema;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "sales_stage", rename_all = "snake_case")]
pub enum SalesStage {
    Prospecting,
    Qualification,
    Proposal,
    Negotiation,
    Closing,
}

impl std::fmt::Display for SalesStage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Prospecting => write!(f, "prospecting"),
            Self::Qualification => write!(f, "qualification"),
            Self::Proposal => write!(f, "proposal"),
            Self::Negotiation => write!(f, "negotiation"),
            Self::Closing => write!(f, "closing"),
        }
    }
}

impl FromStr for SalesStage {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "prospecting" => Ok(Self::Prospecting),
            "qualification" => Ok(Self::Qualification),
            "proposal" => Ok(Self::Proposal),
            "negotiation" => Ok(Self::Negotiation),
            "closing" => Ok(Self::Closing),
            _ => Err(format!("Unknown SalesStage variant: {}", s)),
        }
    }
}

impl Default for SalesStage {
    fn default() -> Self {
        Self::Prospecting
    }
}
