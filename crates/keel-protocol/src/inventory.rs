//! Inventories and their revisions (spec 055 US4, contracts/protocol.md "Inventory").
//!
//! Revisions are immutable: an edit, a restore or a repository change that alters the content
//! makes a new one. Content travels as text: a managed inventory's YAML `source`, and the
//! normalized content as pretty JSON for display and diffs.

use std::fmt;

use keel_core::uuid::Uuid;
use serde::{Deserialize, Deserializer, Serialize};
use ts_rs::TS;

use crate::types::checked_string;

/// Largest inventory (or var schema) text accepted.
pub const MAX_INVENTORY_TEXT_BYTES: usize = 1024 * 1024;

checked_string!(
    /// Inventory YAML, or a var schema as JSON, on its way in. May hold vars an operator did not
    /// mean to show anyone, so `Debug` prints only its length.
    InventoryText,
    "inventory text",
    |v| {
        if v.len() > MAX_INVENTORY_TEXT_BYTES {
            return Err("length");
        }
        Ok(())
    }
);

impl fmt::Debug for InventoryText {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "InventoryText({} bytes)", self.0.len())
    }
}

/// Where an inventory's revisions come from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum InventorySource {
    /// A path in a bound repository; revised by sync.
    Repo,
    /// Edited in Keel.
    Managed,
}

/// `active | deprecated`. A deprecated inventory accepts no new runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum InventoryStatus {
    Active,
    Deprecated,
}

/// `valid | invalid`. An invalid head revision refuses new runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum RevisionStatus {
    Valid,
    Invalid,
}

/// One validation error or warning. Never quotes a var's value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct InventoryProblem {
    pub at: String,
    pub message: String,
}

/// A host and the groups it is in (transitively, without `all`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct InventoryHostInfo {
    pub host: String,
    pub groups: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct InventoryInfo {
    #[ts(type = "string")]
    pub id: Uuid,
    #[ts(type = "string")]
    pub workspace: Uuid,
    pub name: String,
    pub source: InventorySource,
    /// The binding a `repo` inventory comes from.
    #[ts(type = "string | null")]
    pub binding: Option<Uuid>,
    /// The path in that repository.
    pub path: Option<String>,
    pub status: InventoryStatus,
    /// The newest revision's number and status; `None` before the first.
    pub head_seq: Option<u32>,
    pub head_status: Option<RevisionStatus>,
    pub host_count: u32,
    /// Hosts' vars are checked against a schema.
    pub has_schema: bool,
    pub created_at: String,
}

/// A revision as the history lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct InventoryRevisionSummary {
    pub seq: u32,
    pub status: RevisionStatus,
    pub content_hash: String,
    /// The repository commit a `repo` revision was read at.
    pub source_commit: Option<String>,
    /// Who made a managed revision.
    pub author: Option<String>,
    /// The revision this one restored.
    pub restored_from: Option<u32>,
    pub host_count: u32,
    pub created_at: String,
}

/// One revision in full.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct InventoryRevisionInfo {
    pub summary: InventoryRevisionSummary,
    /// A managed inventory's YAML as written.
    pub source: Option<String>,
    /// The normalized content as pretty JSON.
    pub content: String,
    pub hosts: Vec<InventoryHostInfo>,
    pub errors: Vec<InventoryProblem>,
    pub warnings: Vec<InventoryProblem>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum InventoryChangeKind {
    Added,
    Removed,
    Changed,
}

/// One structural difference between two revisions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct InventoryChange {
    /// A dotted path, such as `hosts.web1.vars.http_port`.
    pub path: String,
    pub kind: InventoryChangeKind,
    /// The values as JSON text.
    pub before: Option<String>,
    pub after: Option<String>,
}
