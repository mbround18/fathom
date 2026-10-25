//! Hosted audit log (spec 031, T048/T050): one append-only JSONL file shared by every user's
//! backend, each entry attributed to the signed-in user who made the change.
//!
//! A wrapper (spec 050, T019) over the shared [`audit_log::AuditLog`], which does the writing,
//! reading, permissions and retention:
//!
//! - `<state_dir>/audit/audit-log.jsonl`, directory `0700`, file `0600` (Unix).
//! - One writer mutex for the whole process: every [`UserBackend`](crate::tenant::UserBackend)
//!   holds a clone of the same [`HostedAuditSink`], so concurrent appends never interleave.
//! - Every entry is mirrored to `tracing` (target `fathom::audit`) so a log shipper keeps a
//!   copy outside the pod even with an `emptyDir` state directory.
//! - Retention: [`Retention::CompactAfter`]`(retention)`: once the file holds more than twice
//!   the configured bound it is compacted to the newest `retention` entries.
//! - Corruption tolerance: unparseable lines (e.g. a torn write after a crash) are skipped on
//!   read, and an append after a torn line starts on a fresh line.
//! - Read path: an ordinary user sees only entries whose `actor.sub` is their own; members of
//!   `audit_viewer_groups` see every entry. There is no edit or delete operation.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use audit_log::{AuditLog, AuditOptions, Retention};
use fathom::backend::access::Identity;
use fathom::backend::audit_log::AuditSink;
use fathom::protocol::{AuditActionSummary, AuditActor, AuditLogEntry, AuditOutcome};

/// Default number of entries kept by the hosted audit log (`FATHOM_AUDIT_RETENTION`).
pub const DEFAULT_AUDIT_RETENTION: usize = 10_000;

/// File name under `<state_dir>/audit/`.
pub const AUDIT_FILE: &str = audit_log::AUDIT_FILE;

/// The shared, attributed audit sink of a hosted server process.
pub struct HostedAuditSink {
    log: AuditLog<AuditLogEntry>,
    viewer_groups: Vec<String>,
}

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// Unique id: timestamp nanos plus a process-local counter (as the desktop log does).
fn next_id() -> String {
    let nanos = chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default();
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{nanos:x}-h{n:x}")
}

impl HostedAuditSink {
    /// The sink for a server whose state directory is `state_dir`.
    pub fn new(state_dir: &Path, retention: usize, viewer_groups: Vec<String>) -> Self {
        Self::in_dir(state_dir.join("audit"), retention, viewer_groups)
    }

    /// The sink writing exactly `path` (tests). Its file name must be [`AUDIT_FILE`].
    pub fn at(path: PathBuf, retention: usize, viewer_groups: Vec<String>) -> Self {
        debug_assert_eq!(path.file_name().and_then(|n| n.to_str()), Some(AUDIT_FILE));
        let dir = path.parent().map(Path::to_owned).unwrap_or_default();
        Self::in_dir(dir, retention, viewer_groups)
    }

    fn in_dir(dir: PathBuf, retention: usize, viewer_groups: Vec<String>) -> Self {
        Self {
            log: AuditLog::new(AuditOptions {
                dir,
                retention: Retention::CompactAfter(retention),
            }),
            viewer_groups,
        }
    }

    pub fn path(&self) -> &Path {
        self.log.path()
    }

    /// Whether `viewer` may see every user's entries.
    pub fn sees_all(&self, viewer: &Identity) -> bool {
        viewer
            .groups
            .iter()
            .any(|g| self.viewer_groups.iter().any(|v| v == g))
    }

    /// Append one entry. Errors are returned for tests; [`AuditSink::record`] only logs them.
    pub fn append(&self, entry: &AuditLogEntry) -> std::io::Result<()> {
        self.log.append(entry)
    }

    /// Every readable entry, oldest first. Unparseable lines are skipped.
    pub fn read_all(&self) -> Vec<AuditLogEntry> {
        self.log.all().unwrap_or_else(|e| {
            tracing::warn!("failed to read hosted audit log: {e}");
            Vec::new()
        })
    }

    /// The entries `viewer` may see (T050). No viewer means no hosted user: nothing.
    pub fn visible_to(&self, viewer: Option<&Identity>) -> Vec<AuditLogEntry> {
        let Some(viewer) = viewer else {
            return Vec::new();
        };
        let all = self.read_all();
        if self.sees_all(viewer) {
            return all;
        }
        all.into_iter()
            .filter(|e| e.actor.as_ref().is_some_and(|a| a.sub == viewer.sub))
            .collect()
    }
}

impl AuditSink for HostedAuditSink {
    fn record(
        &self,
        summary: AuditActionSummary,
        context: String,
        request_id: String,
        ok: bool,
        error: Option<&str>,
        actor: Option<AuditActor>,
    ) {
        let entry = AuditLogEntry {
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
        };
        let (actor_sub, actor_name) = entry
            .actor
            .as_ref()
            .map(|a| (a.sub.as_str(), a.username.as_str()))
            .unwrap_or(("", ""));
        tracing::info!(
            target: "fathom::audit",
            id = %entry.id,
            actor.sub = actor_sub,
            actor.username = actor_name,
            action = entry.action_label.as_str(),
            context = entry.context.as_str(),
            target.kind = entry.target_kind.as_deref().unwrap_or(""),
            target.namespace = entry.target_namespace.as_deref().unwrap_or(""),
            target.name = entry.target_name.as_deref().unwrap_or(""),
            outcome = if ok { "success" } else { "failure" },
            request_id = entry.request_id.as_deref().unwrap_or(""),
            "audit"
        );
        if let Err(e) = self.append(&entry) {
            tracing::warn!("failed to record hosted audit entry: {e}");
        }
    }

    fn load(&self, viewer: Option<&Identity>) -> Vec<AuditLogEntry> {
        self.visible_to(viewer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fathom::protocol::AuditActionKind;
    use std::fs;
    use std::sync::Arc;

    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("fathom-hosted-audit-{name}-{}", next_id()));
        dir.join("audit").join(AUDIT_FILE)
    }

    fn summary(name: &str) -> AuditActionSummary {
        AuditActionSummary {
            kind: AuditActionKind::Delete,
            target_kind: Some("Pod".into()),
            target_namespace: Some("team-a".into()),
            target_name: Some(name.into()),
        }
    }

    fn actor(name: &str) -> AuditActor {
        AuditActor {
            sub: format!("sub-{name}"),
            username: name.into(),
        }
    }

    fn who(name: &str, groups: &[&str]) -> Identity {
        Identity {
            sub: format!("sub-{name}"),
            username: name.into(),
            groups: groups.iter().map(|g| (*g).to_owned()).collect(),
            email: None,
        }
    }

    fn cleanup(path: &Path) {
        if let Some(root) = path.parent().and_then(Path::parent) {
            fs::remove_dir_all(root).ok();
        }
    }

    #[test]
    fn records_one_line_per_entry_with_actor() {
        let path = tmp("basic");
        let sink = HostedAuditSink::at(path.clone(), 100, vec![]);
        sink.record(
            summary("p1"),
            "in-cluster".into(),
            "r1".into(),
            true,
            None,
            Some(actor("bob")),
        );
        sink.record(
            summary("p2"),
            "in-cluster".into(),
            String::new(),
            false,
            Some("pods \"p2\" is forbidden"),
            Some(actor("bob")),
        );
        let raw = fs::read_to_string(&path).unwrap();
        assert_eq!(raw.lines().count(), 2);
        let all = sink.read_all();
        assert_eq!(all[0].actor, Some(actor("bob")));
        assert_eq!(all[0].request_id.as_deref(), Some("r1"));
        assert_eq!(all[0].outcome, AuditOutcome::Success);
        assert_eq!(
            all[1].outcome,
            AuditOutcome::Failure {
                reason: "pods \"p2\" is forbidden".into()
            }
        );
        assert!(all[1].request_id.is_none());
        assert!(chrono::DateTime::parse_from_rfc3339(&all[0].timestamp).is_ok());
        cleanup(&path);
    }

    #[cfg(unix)]
    #[test]
    fn file_and_dir_are_private() {
        use std::os::unix::fs::PermissionsExt;
        let path = tmp("mode");
        let sink = HostedAuditSink::at(path.clone(), 100, vec![]);
        sink.append(&entry_for("x", "bob")).unwrap();
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        let dir_mode = fs::metadata(path.parent().unwrap())
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(dir_mode, 0o700);
        cleanup(&path);
    }

    fn entry_for(name: &str, user: &str) -> AuditLogEntry {
        AuditLogEntry {
            id: next_id(),
            timestamp: chrono::Utc::now().to_rfc3339(),
            action: AuditActionKind::Delete,
            action_label: "Delete".into(),
            context: "in-cluster".into(),
            target_kind: Some("Pod".into()),
            target_namespace: Some("team-a".into()),
            target_name: Some(name.into()),
            outcome: AuditOutcome::Success,
            request_id: None,
            actor: Some(actor(user)),
        }
    }

    /// Two backends share one sink and append concurrently: every entry lands on its own
    /// intact line.
    #[test]
    fn concurrent_appends_from_two_backends() {
        let path = tmp("concurrent");
        let sink = Arc::new(HostedAuditSink::at(path.clone(), 10_000, vec![]));
        let per = 200;
        let handles: Vec<_> = ["alice", "bob"]
            .into_iter()
            .map(|user| {
                let sink: Arc<dyn AuditSink> = sink.clone();
                std::thread::spawn(move || {
                    for i in 0..per {
                        sink.record(
                            summary(&format!("{user}-{i}")),
                            "in-cluster".into(),
                            String::new(),
                            true,
                            None,
                            Some(actor(user)),
                        );
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        let raw = fs::read_to_string(&path).unwrap();
        assert_eq!(raw.lines().count(), 2 * per);
        let all = sink.read_all();
        assert_eq!(all.len(), 2 * per);
        for user in ["alice", "bob"] {
            let mine: Vec<_> = all
                .iter()
                .filter(|e| e.actor.as_ref().unwrap().username == user)
                .collect();
            assert_eq!(mine.len(), per);
            // Each writer's own entries stay in order.
            for (i, e) in mine.iter().enumerate() {
                assert_eq!(e.target_name.as_deref(), Some(&*format!("{user}-{i}")));
            }
        }
        cleanup(&path);
    }

    #[test]
    fn tolerates_corrupt_and_torn_lines() {
        let path = tmp("corrupt");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let good = serde_json::to_string(&entry_for("a", "bob")).unwrap();
        // A garbage line, a good line, and a torn final write with no newline.
        fs::write(&path, format!("not json\n{good}\n{{\"id\":\"torn")).unwrap();
        let sink = HostedAuditSink::at(path.clone(), 100, vec![]);
        assert_eq!(sink.read_all().len(), 1);
        sink.append(&entry_for("b", "bob")).unwrap();
        let all = sink.read_all();
        assert_eq!(
            all.len(),
            2,
            "the new entry must not be glued to the torn line"
        );
        assert_eq!(all[1].target_name.as_deref(), Some("b"));
        // A missing file reads as empty.
        assert!(
            HostedAuditSink::at(tmp("missing"), 10, vec![])
                .read_all()
                .is_empty()
        );
        cleanup(&path);
    }

    #[test]
    fn retention_compacts_to_newest() {
        let path = tmp("retention");
        let sink = HostedAuditSink::at(path.clone(), 5, vec![]);
        for i in 0..11 {
            sink.append(&entry_for(&i.to_string(), "bob")).unwrap();
        }
        let all = sink.read_all();
        // Compaction ran once the file passed 2x retention (11 > 10) and kept the newest 5.
        assert_eq!(all.len(), 5);
        assert_eq!(all[0].target_name.as_deref(), Some("6"));
        assert_eq!(all[4].target_name.as_deref(), Some("10"));
        // Appends continue normally after compaction.
        sink.append(&entry_for("11", "bob")).unwrap();
        assert_eq!(sink.read_all().len(), 6);
        cleanup(&path);
    }

    #[test]
    fn visibility_by_actor_and_viewer_groups() {
        let path = tmp("visibility");
        let sink = HostedAuditSink::at(path.clone(), 100, vec!["auditors".into()]);
        sink.append(&entry_for("b1", "bob")).unwrap();
        sink.append(&entry_for("a1", "alice")).unwrap();
        let mut legacy = entry_for("x", "nobody");
        legacy.actor = None;
        sink.append(&legacy).unwrap();

        let bob = sink.load(Some(&who("bob", &["dev"])));
        assert_eq!(bob.len(), 1);
        assert_eq!(bob[0].target_name.as_deref(), Some("b1"));
        let alice = sink.load(Some(&who("alice", &[])));
        assert_eq!(alice.len(), 1);
        assert_eq!(alice[0].target_name.as_deref(), Some("a1"));
        // Same username from a different subject is a different person.
        let mut impostor = who("bob", &[]);
        impostor.sub = "other".into();
        assert!(sink.load(Some(&impostor)).is_empty());
        // Auditors see everything, including entries without an actor.
        assert_eq!(sink.load(Some(&who("carol", &["auditors"]))).len(), 3);
        // No viewer: nothing (never the unfiltered file).
        assert!(sink.load(None).is_empty());
        cleanup(&path);
    }
}
