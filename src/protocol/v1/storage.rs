use serde::{Deserialize, Serialize};

/// Disk usage information for storage resources
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiskUsage {
    /// Namespace (for PVC) or global (for PV)
    pub namespace: String,
    /// Resource name (PVC or PV)
    pub name: String,
    /// Total provisioned bytes
    pub provisioned_bytes: u64,
    /// Used bytes
    pub used_bytes: Option<u64>,
    /// Current state (e.g., "Bound", "Available", "Released")
    pub state: Option<String>,
}
