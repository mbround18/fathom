use serde::{Deserialize, Serialize};
use std::fmt;
use ts_rs::TS;

/// Monotonic request counter. Exported to TypeScript as `number` (safe below 2^53).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
pub struct RequestId(#[ts(type = "number")] pub u64);

impl fmt::Display for RequestId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "req-{}", self.0)
    }
}

/// Monotonic stream counter. Exported to TypeScript as `number` (safe below 2^53).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
pub struct StreamId(#[ts(type = "number")] pub u64);

impl fmt::Display for StreamId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "stream-{}", self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, TS)]
#[serde(tag = "type", content = "data")]
pub enum Correlation {
    #[default]
    None,
    Request(RequestId),
    Stream(StreamId),
}
