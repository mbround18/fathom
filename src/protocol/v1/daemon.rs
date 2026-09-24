//! 036-incluster-daemon: the single Rust definition of the daemon wire contract, shared by
//! `fathom-daemon` and the client. Additive to protocol v1 (no version bump). See
//! `specs/036-incluster-daemon/data-model.md`.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Protocol major the client and daemon must agree on.
pub const DAEMON_PROTOCOL_MAJOR: u32 = 1;
/// Protocol minor this build speaks.
pub const DAEMON_PROTOCOL_MINOR: u32 = 0;

/// Per-context client preference for using the daemon. Missing entry means `Off`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub enum DaemonMode {
    #[default]
    Off,
    Auto,
}

/// Client-side connection state of one context's daemon.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub enum DaemonState {
    Off,
    NotInstalled,
    Probing,
    Available,
    Unavailable,
    NoAccess,
    VersionMismatch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub enum DaemonScopeMode {
    Cluster,
    Namespaces,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct DaemonScope {
    pub mode: DaemonScopeMode,
    pub namespaces: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct DaemonRetention {
    pub max_events: u32,
    pub max_age_secs: u64,
}

/// Whether the daemon can serve pod/node metrics history.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub enum DaemonMetricsAvailability {
    Available,
    MetricsUnavailable,
}

/// Response of `/v1/capabilities`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct DaemonCapabilities {
    pub protocol_major: u32,
    pub protocol_minor: u32,
    pub daemon_version: String,
    /// Random id per process start.
    pub epoch: String,
    pub kinds: Vec<String>,
    pub scope: DaemonScope,
    pub retention: DaemonRetention,
    pub snapshots: Vec<String>,
    pub synced: bool,
    pub topology: bool,
    pub metrics: DaemonMetricsAvailability,
    pub per_user_filtering: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct EventCursor {
    pub epoch: String,
    pub seq: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub enum DerivedOrigin {
    Derived,
    Raw,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub enum DerivedCategory {
    Rollout,
    Condition,
    Scaling,
    Job,
    Cronjob,
    Pod,
    Event,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub enum DerivedSeverity {
    Info,
    Warning,
    Error,
}

/// A controller-level event. `message` is verbatim, capped at 4 KiB, never Secret data.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct DerivedControllerEvent {
    pub origin: DerivedOrigin,
    pub kind: String,
    pub namespace: Option<String>,
    pub name: String,
    pub uid: String,
    pub category: DerivedCategory,
    pub severity: DerivedSeverity,
    pub reason: String,
    pub message: String,
    /// Coalesced count, >= 1.
    pub repeat: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub enum TruncationReason {
    EpochChanged,
    CursorTooOld,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub enum TopologyHealth {
    Ok,
    Warning,
    Error,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub enum TopologyRelation {
    Owns,
    Selects,
    ScheduledOn,
    Scales,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct TopologyNode {
    pub uid: String,
    pub kind: String,
    pub namespace: Option<String>,
    pub name: String,
    pub health: TopologyHealth,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct TopologyEdge {
    pub from: String,
    pub to: String,
    pub relation: TopologyRelation,
}

/// Edge changes within a `topologyDelta`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct EdgeDelta {
    pub added: Vec<TopologyEdge>,
    pub removed: Vec<TopologyEdge>,
}

/// A graph change (or full snapshot with everything in `added`). Carries the shared seq.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct TopologyDelta {
    pub seq: u64,
    pub added: Vec<TopologyNode>,
    /// Removed node uids.
    pub removed: Vec<String>,
    pub edges: EdgeDelta,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub enum MetricKind {
    Cpu,
    Memory,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct SeriesKey {
    pub object_uid: String,
    pub metric: MetricKind,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct DaemonMetricSample {
    /// ISO8601.
    pub time: String,
    pub value: f64,
}

/// A returned series; `truncated_oldest_time` is set when the ring dropped older samples.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct MetricSeries {
    pub key: SeriesKey,
    pub samples: Vec<DaemonMetricSample>,
    pub truncated_oldest_time: Option<String>,
}

/// Namespaces a caller may see, derived via `SubjectAccessReview`; cached <= 60 s. Fail closed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct VisibleScope {
    pub identity_key: String,
    pub namespaces: Vec<String>,
    pub cluster_scoped: bool,
    /// ISO8601.
    pub resolved_at: String,
}

/// One NDJSON line of `/v1/stream`, tagged by `type`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
#[ts(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum DaemonFrame {
    Hello {
        capabilities: DaemonCapabilities,
    },
    Event {
        seq: u64,
        time: String,
        event: DerivedControllerEvent,
    },
    Truncated {
        oldest_seq: u64,
        oldest_time: String,
        dropped: u64,
        reason: TruncationReason,
    },
    Heartbeat {
        seq: u64,
        time: String,
    },
    Bye {
        reason: String,
    },
    ScopeChanged {
        namespaces: Vec<String>,
    },
    TopologyDelta {
        seq: u64,
        added: Vec<TopologyNode>,
        removed: Vec<String>,
        edges: EdgeDelta,
    },
}

/// Truncation marker forwarded to the UI with a timeline batch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct TruncationMarker {
    pub oldest_seq: u64,
    pub oldest_time: String,
    pub dropped: u64,
    pub reason: TruncationReason,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct DaemonTimelineEntry {
    pub seq: u64,
    pub time: String,
    pub event: DerivedControllerEvent,
}

/// Client-side per-context daemon status (`SystemEvent::DaemonStatusUpdated`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct DaemonStatus {
    pub context: String,
    pub state: DaemonState,
    /// Verbatim error, incl. `noAccess`/identity errors.
    pub detail: Option<String>,
    pub capabilities: Option<DaemonCapabilities>,
    pub last_event_at: Option<String>,
    pub next_retry_at: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::de::DeserializeOwned;

    fn rt<T: Serialize + DeserializeOwned + PartialEq + std::fmt::Debug>(v: T) {
        let json = serde_json::to_value(&v).unwrap();
        let back: T = serde_json::from_value(json.clone()).unwrap();
        assert_eq!(back, v);
        assert_eq!(serde_json::to_value(&back).unwrap(), json);
    }

    fn caps() -> DaemonCapabilities {
        DaemonCapabilities {
            protocol_major: 1,
            protocol_minor: 0,
            daemon_version: "0.1.0".into(),
            epoch: "e1".into(),
            kinds: vec!["Deployment".into()],
            scope: DaemonScope {
                mode: DaemonScopeMode::Namespaces,
                namespaces: vec!["a".into()],
            },
            retention: DaemonRetention {
                max_events: 10000,
                max_age_secs: 86400,
            },
            snapshots: vec!["pods".into()],
            synced: true,
            topology: true,
            metrics: DaemonMetricsAvailability::MetricsUnavailable,
            per_user_filtering: true,
        }
    }

    fn ev() -> DerivedControllerEvent {
        DerivedControllerEvent {
            origin: DerivedOrigin::Derived,
            kind: "Deployment".into(),
            namespace: Some("default".into()),
            name: "web".into(),
            uid: "u1".into(),
            category: DerivedCategory::Rollout,
            severity: DerivedSeverity::Error,
            reason: "ProgressDeadlineExceeded".into(),
            message: "m".into(),
            repeat: 2,
        }
    }

    fn node() -> TopologyNode {
        TopologyNode {
            uid: "n1".into(),
            kind: "Pod".into(),
            namespace: None,
            name: "p".into(),
            health: TopologyHealth::Unknown,
        }
    }

    fn edge() -> TopologyEdge {
        TopologyEdge {
            from: "a".into(),
            to: "b".into(),
            relation: TopologyRelation::ScheduledOn,
        }
    }

    #[test]
    fn frames_roundtrip_and_tag() {
        let frames = vec![
            DaemonFrame::Hello {
                capabilities: caps(),
            },
            DaemonFrame::Event {
                seq: 1,
                time: "t".into(),
                event: ev(),
            },
            DaemonFrame::Truncated {
                oldest_seq: 5,
                oldest_time: "t".into(),
                dropped: 4,
                reason: TruncationReason::CursorTooOld,
            },
            DaemonFrame::Heartbeat {
                seq: 2,
                time: "t".into(),
            },
            DaemonFrame::Bye {
                reason: "shutdown".into(),
            },
            DaemonFrame::ScopeChanged {
                namespaces: vec!["a".into()],
            },
            DaemonFrame::TopologyDelta {
                seq: 9,
                added: vec![node()],
                removed: vec!["x".into()],
                edges: EdgeDelta {
                    added: vec![edge()],
                    removed: vec![],
                },
            },
        ];
        let tags = [
            "hello",
            "event",
            "truncated",
            "heartbeat",
            "bye",
            "scopeChanged",
            "topologyDelta",
        ];
        for (f, tag) in frames.into_iter().zip(tags) {
            assert_eq!(serde_json::to_value(&f).unwrap()["type"], tag);
            rt(f);
        }
    }

    #[test]
    fn value_types_roundtrip() {
        rt(caps());
        rt(ev());
        rt(EventCursor {
            epoch: "e".into(),
            seq: 3,
        });
        rt(DaemonMode::Auto);
        rt(node());
        rt(edge());
        rt(TopologyDelta::default());
        rt(SeriesKey {
            object_uid: "u".into(),
            metric: MetricKind::Memory,
        });
        rt(DaemonMetricSample {
            time: "t".into(),
            value: 1.5,
        });
        rt(MetricSeries {
            key: SeriesKey {
                object_uid: "u".into(),
                metric: MetricKind::Cpu,
            },
            samples: vec![DaemonMetricSample {
                time: "t".into(),
                value: 0.25,
            }],
            truncated_oldest_time: Some("t0".into()),
        });
        rt(VisibleScope {
            identity_key: "k".into(),
            namespaces: vec!["a".into()],
            cluster_scoped: false,
            resolved_at: "t".into(),
        });
        rt(TruncationMarker {
            oldest_seq: 1,
            oldest_time: "t".into(),
            dropped: 0,
            reason: TruncationReason::EpochChanged,
        });
        rt(DaemonTimelineEntry {
            seq: 1,
            time: "t".into(),
            event: ev(),
        });
        for s in [
            DaemonState::Off,
            DaemonState::NotInstalled,
            DaemonState::Probing,
            DaemonState::Available,
            DaemonState::Unavailable,
            DaemonState::NoAccess,
            DaemonState::VersionMismatch,
        ] {
            rt(DaemonStatus {
                context: "c".into(),
                state: s,
                detail: Some("d".into()),
                capabilities: Some(caps()),
                last_event_at: None,
                next_retry_at: Some("t".into()),
            });
        }
    }

    #[test]
    fn wire_names_are_camel_case() {
        let v = serde_json::to_value(caps()).unwrap();
        assert_eq!(v["protocolMajor"], 1);
        assert_eq!(v["perUserFiltering"], true);
        assert_eq!(v["metrics"], "metricsUnavailable");
        assert_eq!(
            serde_json::to_value(DerivedCategory::Cronjob).unwrap(),
            "cronjob"
        );
    }
}
