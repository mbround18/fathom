//! Storage interface for metrics history (spec 036 FR-020, R13). Only the in-memory ring
//! ([`crate::metrics_ring::MetricsRing`]) implements it in 036; a durable backend can be added
//! later without protocol changes.

use chrono::{DateTime, Utc};
use fathom::protocol::v1::SeriesKey;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sample {
    pub time: DateTime<Utc>,
    pub value: f64,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct RangeResult {
    pub samples: Vec<Sample>,
    /// Set when the store dropped older samples and the query reaches before what it kept:
    /// the time of the oldest sample it still has. Trends must not be shown as complete.
    pub truncated_oldest: Option<DateTime<Utc>>,
}

pub trait SeriesStore: Send + Sync {
    /// Append a sample. Returns `false` when the store refused it (e.g. byte bound reached).
    fn append(&self, key: &SeriesKey, sample: Sample) -> bool;
    /// Samples with `from <= time <= to`, oldest first.
    fn query_range(&self, key: &SeriesKey, from: DateTime<Utc>, to: DateTime<Utc>) -> RangeResult;
    /// Oldest retained time when older samples were dropped for this series.
    fn truncate_marker(&self, key: &SeriesKey) -> Option<DateTime<Utc>>;
}
