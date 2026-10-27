//! Shared wire types.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// The first frame a client sends on a socket, stating the protocol version it speaks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct Hello {
    pub protocol_version: u32,
}
