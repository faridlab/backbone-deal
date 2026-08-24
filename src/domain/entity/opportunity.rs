use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;
use rust_decimal::Decimal;

use super::OpportunityStatus;
use super::AuditMetadata;

/// Strongly-typed ID for Opportunity
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct OpportunityId(pub Uuid);

impl OpportunityId {
    pub fn new(id: Uuid) -> Self { Self(id) }
    pub fn generate() -> Self { Self(Uuid::new_v4()) }
    pub fn into_inner(self) -> Uuid { self.0 }
}

impl std::fmt::Display for OpportunityId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::str::FromStr for OpportunityId {
    type Err = uuid::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(Uuid::parse_str(s)?))
    }
}

impl From<Uuid> for OpportunityId {
    fn from(id: Uuid) -> Self { Self(id) }
}

impl From<OpportunityId> for Uuid {
    fn from(id: OpportunityId) -> Self { id.0 }
}

impl AsRef<Uuid> for OpportunityId {
    fn as_ref(&self) -> &Uuid { &self.0 }
}

impl std::ops::Deref for OpportunityId {
    type Target = Uuid;
    fn deref(&self) -> &Self::Target { &self.0 }
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Opportunity {
    pub id: Uuid,
    pub company_id: Uuid,
    pub opportunity_name: String,
    pub lead_id: Option<Uuid>,
    pub party_id: Option<Uuid>,
    pub campaign_id: Option<Uuid>,
    pub stage_id: Uuid,
    pub owner_user_id: Option<Uuid>,
    pub sales_team_id: Option<Uuid>,
    pub currency: String,
    pub expected_amount: Decimal,
    pub probability: Decimal,
    pub expected_close_date: Option<DateTime<Utc>>,
    pub status: OpportunityStatus,
    pub quotation_id: Option<Uuid>,
    pub lost_reason: Option<String>,
    pub competitor: Option<String>,
    pub active: bool,
    pub date_last_stage_update: Option<DateTime<Utc>>,
    pub date_closed: Option<DateTime<Utc>>,
    #[serde(default)]
    #[sqlx(json)]
    pub metadata: AuditMetadata,
}

impl Opportunity {
    /// Create a builder for Opportunity
    pub fn builder() -> OpportunityBuilder {
        <OpportunityBuilder as Default>::default()
    }

    /// Create a new Opportunity with required fields
    pub fn new(company_id: Uuid, opportunity_name: String, stage_id: Uuid, currency: String, expected_amount: Decimal, probability: Decimal, status: OpportunityStatus, active: bool) -> Self {
        Self {
            id: Uuid::new_v4(),
            company_id,
            opportunity_name,
            lead_id: None,
            party_id: None,
            campaign_id: None,
            stage_id,
            owner_user_id: None,
            sales_team_id: None,
            currency,
            expected_amount,
            probability,
            expected_close_date: None,
            status,
            quotation_id: None,
            lost_reason: None,
            competitor: None,
            active,
            date_last_stage_update: None,
            date_closed: None,
            metadata: AuditMetadata::default(),
        }
    }

    /// Get the entity's unique identifier
    pub fn id(&self) -> &Uuid {
        &self.id
    }

    /// Get a strongly-typed ID for this entity
    pub fn typed_id(&self) -> OpportunityId {
        OpportunityId(self.id)
    }

    /// Get when this entity was created
    pub fn created_at(&self) -> Option<&DateTime<Utc>> {
        self.metadata.created_at.as_ref()
    }

    /// Get when this entity was last updated
    pub fn updated_at(&self) -> Option<&DateTime<Utc>> {
        self.metadata.updated_at.as_ref()
    }

    /// Check if this entity is soft deleted
    pub fn is_deleted(&self) -> bool {
        self.metadata.deleted_at.is_some()
    }

    /// Check if this entity is active (not deleted)
    pub fn is_active(&self) -> bool {
        self.metadata.deleted_at.is_none()
    }

    /// Get when this entity was deleted
    pub fn deleted_at(&self) -> Option<&DateTime<Utc>> {
        self.metadata.deleted_at.as_ref()
    }

    /// Get who created this entity
    pub fn created_by(&self) -> Option<&Uuid> {
        self.metadata.created_by.as_ref()
    }

    /// Get who last updated this entity
    pub fn updated_by(&self) -> Option<&Uuid> {
        self.metadata.updated_by.as_ref()
    }

    /// Get who deleted this entity
    pub fn deleted_by(&self) -> Option<&Uuid> {
        self.metadata.deleted_by.as_ref()
    }

    /// Get the current status
    pub fn status(&self) -> &OpportunityStatus {
        &self.status
    }


    // ==========================================================
    // Fluent Setters (with_* for optional fields)
    // ==========================================================

    /// Set the lead_id field (chainable)
    pub fn with_lead_id(mut self, value: Uuid) -> Self {
        self.lead_id = Some(value);
        self
    }

    /// Set the party_id field (chainable)
    pub fn with_party_id(mut self, value: Uuid) -> Self {
        self.party_id = Some(value);
        self
    }

    /// Set the campaign_id field (chainable)
    pub fn with_campaign_id(mut self, value: Uuid) -> Self {
        self.campaign_id = Some(value);
        self
    }

    /// Set the owner_user_id field (chainable)
    pub fn with_owner_user_id(mut self, value: Uuid) -> Self {
        self.owner_user_id = Some(value);
        self
    }

    /// Set the sales_team_id field (chainable)
    pub fn with_sales_team_id(mut self, value: Uuid) -> Self {
        self.sales_team_id = Some(value);
        self
    }

    /// Set the expected_close_date field (chainable)
    pub fn with_expected_close_date(mut self, value: DateTime<Utc>) -> Self {
        self.expected_close_date = Some(value);
        self
    }

    /// Set the quotation_id field (chainable)
    pub fn with_quotation_id(mut self, value: Uuid) -> Self {
        self.quotation_id = Some(value);
        self
    }

    /// Set the lost_reason field (chainable)
    pub fn with_lost_reason(mut self, value: String) -> Self {
        self.lost_reason = Some(value);
        self
    }

    /// Set the competitor field (chainable)
    pub fn with_competitor(mut self, value: String) -> Self {
        self.competitor = Some(value);
        self
    }

    /// Set the date_last_stage_update field (chainable)
    pub fn with_date_last_stage_update(mut self, value: DateTime<Utc>) -> Self {
        self.date_last_stage_update = Some(value);
        self
    }

    /// Set the date_closed field (chainable)
    pub fn with_date_closed(mut self, value: DateTime<Utc>) -> Self {
        self.date_closed = Some(value);
        self
    }

    // ==========================================================
    // Partial Update
    // ==========================================================

    /// Apply partial updates from a map of field name to JSON value
    pub fn apply_patch(&mut self, fields: std::collections::HashMap<String, serde_json::Value>) {
        for (key, value) in fields {
            match key.as_str() {
                "company_id" => {
                    if let Ok(v) = serde_json::from_value(value) { self.company_id = v; }
                }
                "opportunity_name" => {
                    if let Ok(v) = serde_json::from_value(value) { self.opportunity_name = v; }
                }
                "lead_id" => {
                    if let Ok(v) = serde_json::from_value(value) { self.lead_id = v; }
                }
                "party_id" => {
                    if let Ok(v) = serde_json::from_value(value) { self.party_id = v; }
                }
                "campaign_id" => {
                    if let Ok(v) = serde_json::from_value(value) { self.campaign_id = v; }
                }
                "stage_id" => {
                    if let Ok(v) = serde_json::from_value(value) { self.stage_id = v; }
                }
                "owner_user_id" => {
                    if let Ok(v) = serde_json::from_value(value) { self.owner_user_id = v; }
                }
                "sales_team_id" => {
                    if let Ok(v) = serde_json::from_value(value) { self.sales_team_id = v; }
                }
                "currency" => {
                    if let Ok(v) = serde_json::from_value(value) { self.currency = v; }
                }
                "expected_amount" => {
                    if let Ok(v) = serde_json::from_value(value) { self.expected_amount = v; }
                }
                "probability" => {
                    if let Ok(v) = serde_json::from_value(value) { self.probability = v; }
                }
                "expected_close_date" => {
                    if let Ok(v) = serde_json::from_value(value) { self.expected_close_date = v; }
                }
                "status" => {
                    if let Ok(v) = serde_json::from_value(value) { self.status = v; }
                }
                "quotation_id" => {
                    if let Ok(v) = serde_json::from_value(value) { self.quotation_id = v; }
                }
                "lost_reason" => {
                    if let Ok(v) = serde_json::from_value(value) { self.lost_reason = v; }
                }
                "competitor" => {
                    if let Ok(v) = serde_json::from_value(value) { self.competitor = v; }
                }
                "active" => {
                    if let Ok(v) = serde_json::from_value(value) { self.active = v; }
                }
                "date_last_stage_update" => {
                    if let Ok(v) = serde_json::from_value(value) { self.date_last_stage_update = v; }
                }
                "date_closed" => {
                    if let Ok(v) = serde_json::from_value(value) { self.date_closed = v; }
                }
                _ => {} // ignore unknown fields
            }
        }
    }

    // <<< CUSTOM METHODS START >>>
    // <<< CUSTOM METHODS END >>>
}

impl super::Entity for Opportunity {
    type Id = Uuid;

    fn entity_id(&self) -> &Self::Id {
        &self.id
    }

    fn entity_type() -> &'static str {
        "Opportunity"
    }
}

impl backbone_core::PersistentEntity for Opportunity {
    fn entity_id(&self) -> String {
        self.id.to_string()
    }
    fn set_entity_id(&mut self, id: String) {
        if let Ok(uuid) = uuid::Uuid::parse_str(&id) {
            self.id = uuid;
        }
    }
    fn created_at(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        self.metadata.created_at
    }
    fn set_created_at(&mut self, ts: chrono::DateTime<chrono::Utc>) {
        self.metadata.created_at = Some(ts);
    }
    fn updated_at(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        self.metadata.updated_at
    }
    fn set_updated_at(&mut self, ts: chrono::DateTime<chrono::Utc>) {
        self.metadata.updated_at = Some(ts);
    }
    fn deleted_at(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        self.metadata.deleted_at
    }
    fn set_deleted_at(&mut self, ts: Option<chrono::DateTime<chrono::Utc>>) {
        self.metadata.deleted_at = ts;
    }
}

impl backbone_orm::EntityRepoMeta for Opportunity {
    fn column_types() -> std::collections::HashMap<String, String> {
        let mut m = std::collections::HashMap::new();
        m.insert("id".to_string(), "uuid".to_string());
        m.insert("company_id".to_string(), "uuid".to_string());
        m.insert("lead_id".to_string(), "uuid".to_string());
        m.insert("party_id".to_string(), "uuid".to_string());
        m.insert("campaign_id".to_string(), "uuid".to_string());
        m.insert("stage_id".to_string(), "uuid".to_string());
        m.insert("owner_user_id".to_string(), "uuid".to_string());
        m.insert("sales_team_id".to_string(), "uuid".to_string());
        m.insert("quotation_id".to_string(), "uuid".to_string());
        m.insert("status".to_string(), "opportunity_status".to_string());
        m
    }
    fn search_fields() -> &'static [&'static str] {
        &["opportunity_name", "currency"]
    }
    fn company_field() -> Option<&'static str> {
        Some("company_id")
    }
}

/// Builder for Opportunity entity
///
/// Provides a fluent API for constructing Opportunity instances.
/// System fields (id, metadata, timestamps) are auto-initialized.
#[derive(Debug, Clone, Default)]
pub struct OpportunityBuilder {
    company_id: Option<Uuid>,
    opportunity_name: Option<String>,
    lead_id: Option<Uuid>,
    party_id: Option<Uuid>,
    campaign_id: Option<Uuid>,
    stage_id: Option<Uuid>,
    owner_user_id: Option<Uuid>,
    sales_team_id: Option<Uuid>,
    currency: Option<String>,
    expected_amount: Option<Decimal>,
    probability: Option<Decimal>,
    expected_close_date: Option<DateTime<Utc>>,
    status: Option<OpportunityStatus>,
    quotation_id: Option<Uuid>,
    lost_reason: Option<String>,
    competitor: Option<String>,
    active: Option<bool>,
    date_last_stage_update: Option<DateTime<Utc>>,
    date_closed: Option<DateTime<Utc>>,
}

impl OpportunityBuilder {
    /// Set the company_id field (required)
    pub fn company_id(mut self, value: Uuid) -> Self {
        self.company_id = Some(value);
        self
    }

    /// Set the opportunity_name field (required)
    pub fn opportunity_name(mut self, value: String) -> Self {
        self.opportunity_name = Some(value);
        self
    }

    /// Set the lead_id field (optional)
    pub fn lead_id(mut self, value: Uuid) -> Self {
        self.lead_id = Some(value);
        self
    }

    /// Set the party_id field (optional)
    pub fn party_id(mut self, value: Uuid) -> Self {
        self.party_id = Some(value);
        self
    }

    /// Set the campaign_id field (optional)
    pub fn campaign_id(mut self, value: Uuid) -> Self {
        self.campaign_id = Some(value);
        self
    }

    /// Set the stage_id field (required)
    pub fn stage_id(mut self, value: Uuid) -> Self {
        self.stage_id = Some(value);
        self
    }

    /// Set the owner_user_id field (optional)
    pub fn owner_user_id(mut self, value: Uuid) -> Self {
        self.owner_user_id = Some(value);
        self
    }

    /// Set the sales_team_id field (optional)
    pub fn sales_team_id(mut self, value: Uuid) -> Self {
        self.sales_team_id = Some(value);
        self
    }

    /// Set the currency field (default: `"IDR".to_string()`)
    pub fn currency(mut self, value: String) -> Self {
        self.currency = Some(value);
        self
    }

    /// Set the expected_amount field (default: `Decimal::from(0)`)
    pub fn expected_amount(mut self, value: Decimal) -> Self {
        self.expected_amount = Some(value);
        self
    }

    /// Set the probability field (default: `Decimal::from(0)`)
    pub fn probability(mut self, value: Decimal) -> Self {
        self.probability = Some(value);
        self
    }

    /// Set the expected_close_date field (optional)
    pub fn expected_close_date(mut self, value: DateTime<Utc>) -> Self {
        self.expected_close_date = Some(value);
        self
    }

    /// Set the status field (default: `OpportunityStatus::default()`)
    pub fn status(mut self, value: OpportunityStatus) -> Self {
        self.status = Some(value);
        self
    }

    /// Set the quotation_id field (optional)
    pub fn quotation_id(mut self, value: Uuid) -> Self {
        self.quotation_id = Some(value);
        self
    }

    /// Set the lost_reason field (optional)
    pub fn lost_reason(mut self, value: String) -> Self {
        self.lost_reason = Some(value);
        self
    }

    /// Set the competitor field (optional)
    pub fn competitor(mut self, value: String) -> Self {
        self.competitor = Some(value);
        self
    }

    /// Set the active field (default: `true`)
    pub fn active(mut self, value: bool) -> Self {
        self.active = Some(value);
        self
    }

    /// Set the date_last_stage_update field (optional)
    pub fn date_last_stage_update(mut self, value: DateTime<Utc>) -> Self {
        self.date_last_stage_update = Some(value);
        self
    }

    /// Set the date_closed field (optional)
    pub fn date_closed(mut self, value: DateTime<Utc>) -> Self {
        self.date_closed = Some(value);
        self
    }

    /// Build the Opportunity entity
    ///
    /// Returns Err if any required field without a default is missing.
    pub fn build(self) -> Result<Opportunity, String> {
        let company_id = self.company_id.ok_or_else(|| "company_id is required".to_string())?;
        let opportunity_name = self.opportunity_name.ok_or_else(|| "opportunity_name is required".to_string())?;
        let stage_id = self.stage_id.ok_or_else(|| "stage_id is required".to_string())?;

        Ok(Opportunity {
            id: Uuid::new_v4(),
            company_id,
            opportunity_name,
            lead_id: self.lead_id,
            party_id: self.party_id,
            campaign_id: self.campaign_id,
            stage_id,
            owner_user_id: self.owner_user_id,
            sales_team_id: self.sales_team_id,
            currency: self.currency.unwrap_or("IDR".to_string()),
            expected_amount: self.expected_amount.unwrap_or(Decimal::from(0)),
            probability: self.probability.unwrap_or(Decimal::from(0)),
            expected_close_date: self.expected_close_date,
            status: self.status.unwrap_or_default(),
            quotation_id: self.quotation_id,
            lost_reason: self.lost_reason,
            competitor: self.competitor,
            active: self.active.unwrap_or(true),
            date_last_stage_update: self.date_last_stage_update,
            date_closed: self.date_closed,
            metadata: AuditMetadata::default(),
        })
    }
}
