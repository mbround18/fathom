use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::route::SectionKey;

/// How Fathom attributed a synthesized `Application` to its owning install mechanism.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
pub enum ApplicationManagedBy {
    Helm,
    Flux,
    LabelHeuristic,
}

/// One row in the Applications list (synthesized, not a single live Kubernetes object).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct ApplicationSummary {
    pub instance: String,
    pub name: String,
    pub namespace: String,
    pub managed_by: ApplicationManagedBy,
    pub version: Option<String>,
    pub creation_timestamp: String,
    pub ready_count: i32,
    pub total_count: i32,
}

/// Full detail for one selected application.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct ApplicationDetails {
    pub instance: String,
    pub name: String,
    pub namespace: String,
    pub managed_by: ApplicationManagedBy,
    pub version: Option<String>,
    pub creation_timestamp: String,
    pub labels: BTreeMap<String, String>,
    pub urls: Vec<String>,
    pub related: Vec<RelatedResourceRef>,
}

/// A link from one resource's inspector to another related resource, produced by the
/// generic related-resources resolver (`src/backend/related.rs`).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct RelatedResourceRef {
    pub section: SectionKey,
    pub kind: String,
    pub namespace: Option<String>,
    pub name: String,
    pub relation: String,
    pub status_hint: Option<String>,
}
