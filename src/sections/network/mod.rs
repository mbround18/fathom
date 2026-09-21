// Network section tree: Services, Ingress, NetworkPolicies, and Port Forwards.
//!
//! This module manages network resources and port forwarding between local and cluster.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct PortForward {
    pub id: String,
    pub namespace: String,
    pub pod: String,
    pub local_port: u16,
    pub remote_port: u16,
    pub container: Option<String>,
    pub status: PortForwardStatus,
    #[serde(default)]
    pub auto_start: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
pub enum PortForwardStatus {
    Connecting,
    Active,
    Error,
    Closed,
}
