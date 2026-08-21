use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

use super::CampaignStatus;
use super::AuditMetadata;

/// Strongly-typed ID for Campaign
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CampaignId(pub Uuid);

impl CampaignId {
    pub fn new(id: Uuid) -> Self { Self(id) }
    pub fn generate() -> Self { Self(Uuid::new_v4()) }
    pub fn into_inner(self) -> Uuid { self.0 }
}

impl std::fmt::Display for CampaignId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::str::FromStr for CampaignId {
    type Err = uuid::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(Uuid::parse_str(s)?))
    }
}

impl From<Uuid> for CampaignId {
    fn from(id: Uuid) -> Self { Self(id) }
}

impl From<CampaignId> for Uuid {
    fn from(id: CampaignId) -> Self { id.0 }
}

impl AsRef<Uuid> for CampaignId {
    fn as_ref(&self) -> &Uuid { &self.0 }
}

impl std::ops::Deref for CampaignId {
    type Target = Uuid;
    fn deref(&self) -> &Self::Target { &self.0 }
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Campaign {
    pub id: Uuid,
    pub company_id: Uuid,
    pub campaign_name: String,
    pub utm_source: Option<String>,
    pub utm_medium: Option<String>,
    pub utm_campaign: Option<String>,
    pub status: CampaignStatus,
    #[serde(default)]
    #[sqlx(json)]
    pub metadata: AuditMetadata,
}

impl Campaign {
    /// Create a builder for Campaign
    pub fn builder() -> CampaignBuilder {
        <CampaignBuilder as Default>::default()
    }

    /// Create a new Campaign with required fields
    pub fn new(company_id: Uuid, campaign_name: String, status: CampaignStatus) -> Self {
        Self {
            id: Uuid::new_v4(),
            company_id,
            campaign_name,
            utm_source: None,
            utm_medium: None,
            utm_campaign: None,
            status,
            metadata: AuditMetadata::default(),
        }
    }

    /// Get the entity's unique identifier
    pub fn id(&self) -> &Uuid {
        &self.id
    }

    /// Get a strongly-typed ID for this entity
    pub fn typed_id(&self) -> CampaignId {
        CampaignId(self.id)
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
    pub fn status(&self) -> &CampaignStatus {
        &self.status
    }


    // ==========================================================
    // Fluent Setters (with_* for optional fields)
    // ==========================================================

    /// Set the utm_source field (chainable)
    pub fn with_utm_source(mut self, value: String) -> Self {
        self.utm_source = Some(value);
        self
    }

    /// Set the utm_medium field (chainable)
    pub fn with_utm_medium(mut self, value: String) -> Self {
        self.utm_medium = Some(value);
        self
    }

    /// Set the utm_campaign field (chainable)
    pub fn with_utm_campaign(mut self, value: String) -> Self {
        self.utm_campaign = Some(value);
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
                "campaign_name" => {
                    if let Ok(v) = serde_json::from_value(value) { self.campaign_name = v; }
                }
                "utm_source" => {
                    if let Ok(v) = serde_json::from_value(value) { self.utm_source = v; }
                }
                "utm_medium" => {
                    if let Ok(v) = serde_json::from_value(value) { self.utm_medium = v; }
                }
                "utm_campaign" => {
                    if let Ok(v) = serde_json::from_value(value) { self.utm_campaign = v; }
                }
                "status" => {
                    if let Ok(v) = serde_json::from_value(value) { self.status = v; }
                }
                _ => {} // ignore unknown fields
            }
        }
    }

    // <<< CUSTOM METHODS START >>>
    // <<< CUSTOM METHODS END >>>
}

impl super::Entity for Campaign {
    type Id = Uuid;

    fn entity_id(&self) -> &Self::Id {
        &self.id
    }

    fn entity_type() -> &'static str {
        "Campaign"
    }
}

impl backbone_core::PersistentEntity for Campaign {
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

impl backbone_orm::EntityRepoMeta for Campaign {
    fn column_types() -> std::collections::HashMap<String, String> {
        let mut m = std::collections::HashMap::new();
        m.insert("id".to_string(), "uuid".to_string());
        m.insert("company_id".to_string(), "uuid".to_string());
        m.insert("status".to_string(), "campaign_status".to_string());
        m
    }
    fn search_fields() -> &'static [&'static str] {
        &["campaign_name"]
    }
    fn company_field() -> Option<&'static str> {
        Some("company_id")
    }
}

/// Builder for Campaign entity
///
/// Provides a fluent API for constructing Campaign instances.
/// System fields (id, metadata, timestamps) are auto-initialized.
#[derive(Debug, Clone, Default)]
pub struct CampaignBuilder {
    company_id: Option<Uuid>,
    campaign_name: Option<String>,
    utm_source: Option<String>,
    utm_medium: Option<String>,
    utm_campaign: Option<String>,
    status: Option<CampaignStatus>,
}

impl CampaignBuilder {
    /// Set the company_id field (required)
    pub fn company_id(mut self, value: Uuid) -> Self {
        self.company_id = Some(value);
        self
    }

    /// Set the campaign_name field (required)
    pub fn campaign_name(mut self, value: String) -> Self {
        self.campaign_name = Some(value);
        self
    }

    /// Set the utm_source field (optional)
    pub fn utm_source(mut self, value: String) -> Self {
        self.utm_source = Some(value);
        self
    }

    /// Set the utm_medium field (optional)
    pub fn utm_medium(mut self, value: String) -> Self {
        self.utm_medium = Some(value);
        self
    }

    /// Set the utm_campaign field (optional)
    pub fn utm_campaign(mut self, value: String) -> Self {
        self.utm_campaign = Some(value);
        self
    }

    /// Set the status field (default: `CampaignStatus::default()`)
    pub fn status(mut self, value: CampaignStatus) -> Self {
        self.status = Some(value);
        self
    }

    /// Build the Campaign entity
    ///
    /// Returns Err if any required field without a default is missing.
    pub fn build(self) -> Result<Campaign, String> {
        let company_id = self.company_id.ok_or_else(|| "company_id is required".to_string())?;
        let campaign_name = self.campaign_name.ok_or_else(|| "campaign_name is required".to_string())?;

        Ok(Campaign {
            id: Uuid::new_v4(),
            company_id,
            campaign_name,
            utm_source: self.utm_source,
            utm_medium: self.utm_medium,
            utm_campaign: self.utm_campaign,
            status: self.status.unwrap_or_default(),
            metadata: AuditMetadata::default(),
        })
    }
}
