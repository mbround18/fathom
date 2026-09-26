//! Persistence for the local mutation audit log (`~/.fathom/audit-log.json`), spec
//! `026-mutation-audit-log-viewer`. Append-only, bounded to [`AUDIT_LOG_RETENTION`] entries
//! (oldest evicted first). Write failures are logged and never surfaced to the command that
//! triggered them (FR-011); a missing or corrupt file reads as empty and is left untouched.

use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::Result;
use serde::{Deserialize, Serialize};

use super::access::Identity;
use super::persistence;
use crate::protocol::{
    AUDIT_LOG_RETENTION, AuditActionSummary, AuditActor, AuditLogEntry, AuditOutcome,
};

/// Where completed mutating commands are recorded (031-hosted-server-mode). The default is
/// [`LocalFileSink`] (the desktop `~/.fathom/audit-log.json`); the hosted server supplies a
/// shared, attributed sink. Implementations must never fail the caller.
pub trait AuditSink: Send + Sync {
    fn record(
        &self,
        summary: AuditActionSummary,
        context: String,
        request_id: String,
        ok: bool,
        error: Option<&str>,
        actor: Option<AuditActor>,
    );

    /// Entries `viewer` may see, oldest first (the audit viewer's read path). `viewer` is the
    /// signed-in user behind the backend (`None` on desktop). The default reads the local
    /// file unfiltered: on desktop every entry is the operator's own. There is deliberately no
    /// edit or delete operation.
    fn load(&self, viewer: Option<&Identity>) -> Vec<AuditLogEntry> {
        let _ = viewer;
        load_entries()
    }
}

/// Today's behavior: append to the local audit-log file.
#[derive(Debug, Clone, Copy, Default)]
pub struct LocalFileSink;

impl AuditSink for LocalFileSink {
    fn record(
        &self,
        summary: AuditActionSummary,
        context: String,
        request_id: String,
        ok: bool,
        error: Option<&str>,
        actor: Option<AuditActor>,
    ) {
        record(summary, context, request_id, ok, error, actor);
    }
}

#[derive(Default, Serialize, Deserialize)]
struct AuditLogStore {
    entries: Vec<AuditLogEntry>,
}

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// Unique-enough local id: timestamp nanos, pid and a process-local counter (no UUID dependency).
fn next_id() -> String {
    let nanos = chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default();
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{nanos:x}-{:x}-{n:x}", std::process::id())
}

/// Read entries from `path`; missing/corrupt -> empty, file untouched.
pub fn load_entries_from(path: &Path) -> Vec<AuditLogEntry> {
    let Ok(content) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    match serde_json::from_str::<AuditLogStore>(&content) {
        Ok(store) => store.entries,
        Err(e) => {
            tracing::warn!("audit-log.json is not readable ({e}); treating history as empty");
            Vec::new()
        }
    }
}

/// Append `entry` to `path`, evicting oldest entries beyond the retention bound. Refuses to
/// overwrite an existing file that cannot be parsed.
pub fn append_entry_to(path: &Path, entry: AuditLogEntry) -> Result<()> {
    let mut entries = match std::fs::read_to_string(path) {
        Ok(content) => {
            serde_json::from_str::<AuditLogStore>(&content)
                .map_err(|e| anyhow::anyhow!("existing audit-log.json is unreadable: {e}"))?
                .entries
        }
        Err(_) => Vec::new(),
    };
    entries.push(entry);
    if entries.len() > AUDIT_LOG_RETENTION {
        let excess = entries.len() - AUDIT_LOG_RETENTION;
        entries.drain(..excess);
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(
        path,
        serde_json::to_string_pretty(&AuditLogStore { entries })?,
    )?;
    Ok(())
}

pub fn load_entries() -> Vec<AuditLogEntry> {
    match persistence::audit_log_path() {
        Ok(p) => load_entries_from(&p),
        Err(_) => Vec::new(),
    }
}

/// Build the entry for one completed mutating command.
fn build_entry(
    summary: AuditActionSummary,
    context: String,
    request_id: String,
    ok: bool,
    error: Option<&str>,
    actor: Option<AuditActor>,
) -> AuditLogEntry {
    AuditLogEntry {
        id: next_id(),
        timestamp: chrono::Utc::now().to_rfc3339(),
        action: summary.kind,
        action_label: summary.kind.label().to_owned(),
        context,
        target_kind: summary.target_kind,
        target_namespace: summary.target_namespace,
        target_name: summary.target_name,
        outcome: AuditOutcome::from_result(ok, error),
        request_id: (!request_id.is_empty()).then_some(request_id),
        actor,
    }
}

/// Record one completed mutating command. Never fails the caller.
pub fn record(
    summary: AuditActionSummary,
    context: String,
    request_id: String,
    ok: bool,
    error: Option<&str>,
    actor: Option<AuditActor>,
) {
    let entry = build_entry(summary, context, request_id, ok, error, actor);
    let result = persistence::audit_log_path().and_then(|p| append_entry_to(&p, entry));
    if let Err(e) = result {
        tracing::warn!("Failed to record audit-log entry: {e:#}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::AuditActionKind;

    fn entry(i: usize) -> AuditLogEntry {
        AuditLogEntry {
            id: i.to_string(),
            timestamp: "2026-01-01T00:00:00Z".into(),
            action: AuditActionKind::NodeCordon,
            action_label: "Node Cordon".into(),
            context: "ctx".into(),
            target_kind: Some("Node".into()),
            target_namespace: None,
            target_name: Some(format!("n{i}")),
            outcome: AuditOutcome::Success,
            request_id: None,
            actor: None,
        }
    }

    #[test]
    fn round_trip_and_retention() {
        let dir = std::env::temp_dir().join(format!("fathom-audit-{}", next_id()));
        let path = dir.join("audit-log.json");
        for i in 0..(AUDIT_LOG_RETENTION + 1) {
            append_entry_to(&path, entry(i)).unwrap();
        }
        let got = load_entries_from(&path);
        assert_eq!(got.len(), AUDIT_LOG_RETENTION);
        assert_eq!(got.first().unwrap().id, "1");
        assert_eq!(got.last().unwrap().id, AUDIT_LOG_RETENTION.to_string());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn corrupt_file_is_empty_and_untouched() {
        let dir = std::env::temp_dir().join(format!("fathom-audit-{}", next_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("audit-log.json");
        std::fs::write(&path, "not json").unwrap();
        assert!(load_entries_from(&path).is_empty());
        assert!(append_entry_to(&path, entry(1)).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "not json");
        assert!(load_entries_from(&dir.join("missing.json")).is_empty());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn legacy_file_without_actor_loads() {
        let dir = std::env::temp_dir().join(format!("fathom-audit-{}", next_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("audit-log.json");
        std::fs::write(
            &path,
            r#"{"entries":[{"id":"a","timestamp":"t","action":"nodeCordon","actionLabel":"Node Cordon","context":"c","targetKind":null,"targetNamespace":null,"targetName":null,"outcome":{"type":"Success"},"requestId":null}]}"#,
        )
        .unwrap();
        let got = load_entries_from(&path);
        assert_eq!(got.len(), 1);
        assert!(got[0].actor.is_none());
        let mut with_actor = entry(2);
        with_actor.actor = Some(AuditActor {
            sub: "s".into(),
            username: "alice".into(),
        });
        append_entry_to(&path, with_actor.clone()).unwrap();
        assert_eq!(load_entries_from(&path)[1], with_actor);
        std::fs::remove_dir_all(dir).ok();
    }

    /// T053 (FR-018, Principle VI): the desktop default sink is unchanged. Entries carry no
    /// actor, the file on disk has no `actor` key at all (byte-compatible with pre-031
    /// readers), a pre-031 file keeps loading, and the default read path is unfiltered.
    #[test]
    fn desktop_default_sink_writes_without_actor_and_loads_legacy() {
        let dir = std::env::temp_dir().join(format!("fathom-audit-desktop-{}", next_id()));
        std::fs::create_dir_all(&dir).unwrap();
        // A pre-031 file, exactly as spec 026 wrote it.
        std::fs::write(
            dir.join("audit-log.json"),
            r#"{"entries":[{"id":"legacy","timestamp":"t","action":"scale","actionLabel":"Scale","context":"kind-dev","targetKind":"Deployment","targetNamespace":"default","targetName":"web","outcome":{"type":"Success"},"requestId":"r0"}]}"#,
        )
        .unwrap();
        let root = dir.clone();
        let (loaded, raw) = std::thread::spawn(move || {
            // The state root is scoped to this thread, like one backend's threads.
            persistence::set_thread_state_root(root.clone());
            let sink: &dyn AuditSink = &LocalFileSink;
            sink.record(
                AuditActionSummary {
                    kind: AuditActionKind::Delete,
                    target_kind: Some("Pod".into()),
                    target_namespace: Some("default".into()),
                    target_name: Some("web-0".into()),
                },
                "kind-dev".into(),
                "r1".into(),
                false,
                Some("pods \"web-0\" is forbidden"),
                None,
            );
            (
                sink.load(None),
                std::fs::read_to_string(root.join("audit-log.json")).unwrap(),
            )
        })
        .join()
        .unwrap();
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].id, "legacy");
        assert!(loaded.iter().all(|e| e.actor.is_none()));
        assert_eq!(loaded[1].target_name.as_deref(), Some("web-0"));
        assert_eq!(
            loaded[1].outcome,
            AuditOutcome::Failure {
                reason: "pods \"web-0\" is forbidden".into()
            }
        );
        assert!(
            !raw.contains("actor"),
            "desktop entries must not gain an actor key"
        );
        let json = serde_json::to_value(build_entry(
            AuditActionSummary {
                kind: AuditActionKind::NodeCordon,
                target_kind: None,
                target_namespace: None,
                target_name: Some("n1".into()),
            },
            "ctx".into(),
            String::new(),
            true,
            None,
            None,
        ))
        .unwrap();
        assert!(json.get("actor").is_none());
        assert!(json["requestId"].is_null());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn failure_without_message_gets_placeholder() {
        assert_eq!(
            AuditOutcome::from_result(false, None),
            AuditOutcome::Failure {
                reason: "Unknown error".into()
            }
        );
    }
}
