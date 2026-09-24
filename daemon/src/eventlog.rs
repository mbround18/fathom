//! Bounded in-memory, sequence-numbered event log with replay (spec 036 R5, FR-004).
//!
//! Invariants: `seq` starts at 1, is strictly increasing and contiguous within an epoch;
//! eviction is by count or age; a cursor outside retention (or from another epoch) yields an
//! explicit [`Truncation`], never a silent gap.

use crate::topology::ScopedDelta;
use chrono::{DateTime, Duration, Utc};
use fathom::protocol::v1::{DerivedControllerEvent, TruncationReason};
use std::collections::VecDeque;

/// One log record's content. Topology deltas share the sequence space with events.
#[derive(Debug, Clone)]
pub enum Payload {
    Event(DerivedControllerEvent),
    Topology(ScopedDelta),
}

#[derive(Debug, Clone)]
pub struct Entry {
    pub seq: u64,
    pub time: DateTime<Utc>,
    pub payload: Payload,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Truncation {
    pub oldest_seq: u64,
    pub oldest_time: DateTime<Utc>,
    pub dropped: u64,
    pub reason: TruncationReason,
}

#[derive(Debug, Default)]
pub struct Replay {
    pub truncated: Option<Truncation>,
    pub entries: Vec<Entry>,
}

pub struct EventLog {
    epoch: String,
    max_events: usize,
    max_age: Duration,
    last_seq: u64,
    entries: VecDeque<Entry>,
}

impl EventLog {
    pub fn new(epoch: String, max_events: usize, max_age: Duration) -> Self {
        Self {
            epoch,
            max_events: max_events.max(1),
            max_age,
            last_seq: 0,
            entries: VecDeque::new(),
        }
    }

    pub fn epoch(&self) -> &str {
        &self.epoch
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn oldest_seq(&self) -> Option<u64> {
        self.entries.front().map(|e| e.seq)
    }

    pub fn last_seq(&self) -> u64 {
        self.last_seq
    }

    pub fn evict(&mut self, now: DateTime<Utc>) {
        while self.entries.len() > self.max_events {
            self.entries.pop_front();
        }
        while self
            .entries
            .front()
            .is_some_and(|e| now.signed_duration_since(e.time) > self.max_age)
        {
            self.entries.pop_front();
        }
    }

    pub fn append(&mut self, payload: Payload, now: DateTime<Utc>) -> Entry {
        self.last_seq += 1;
        let e = Entry {
            seq: self.last_seq,
            time: now,
            payload,
        };
        self.entries.push_back(e.clone());
        self.evict(now);
        e
    }

    /// Replay for a resume request. `after == None` is live-only. A truncation is followed by
    /// everything still retained so the client loses nothing that survives.
    pub fn replay(&self, epoch: Option<&str>, after: Option<u64>, now: DateTime<Utc>) -> Replay {
        let Some(after) = after else {
            return Replay::default();
        };
        let oldest_seq = self.oldest_seq().unwrap_or(self.last_seq + 1);
        let oldest_time = self.entries.front().map_or(now, |e| e.time);
        let trunc = |reason, dropped| Truncation {
            oldest_seq,
            oldest_time,
            dropped,
            reason,
        };
        let retained = |from: u64| {
            self.entries
                .iter()
                .filter(|e| e.seq > from)
                .cloned()
                .collect()
        };
        if epoch != Some(self.epoch.as_str()) {
            return Replay {
                truncated: Some(trunc(TruncationReason::EpochChanged, 0)),
                entries: retained(0),
            };
        }
        if after > self.last_seq {
            return Replay {
                truncated: Some(trunc(TruncationReason::CursorTooOld, 0)),
                entries: retained(0),
            };
        }
        if after + 1 < oldest_seq {
            return Replay {
                truncated: Some(trunc(
                    TruncationReason::CursorTooOld,
                    oldest_seq - 1 - after,
                )),
                entries: retained(0),
            };
        }
        Replay {
            truncated: None,
            entries: retained(after),
        }
    }
}
