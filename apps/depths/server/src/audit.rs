//! T040: the Depths audit log (data-model "AuditEntry"), one JSON line per entry under
//! `<data-dir>/audit/`, rotated by size.
//!
//! An entry names who did what to which stack, the argv that ran and how it ended. It never holds
//! file content or `.env` values (FR-021, FR-035): the builders here take names, argv and docker's
//! stderr tail only, and nothing in the server passes a file's text in.

use std::net::IpAddr;
use std::path::{Path, PathBuf};

use audit_log::{AuditLog, AuditOptions, Cursor, Retention};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use depths_protocol::{
    Actor, AuditAction, AuditCursor, AuditEntry, AuditFilter, AuditOutcome, OperationId, PreviewId,
    UserChange,
};

use crate::users::provider_str;

/// Entries per `GetAudit` page.
pub const PAGE: usize = 100;

pub struct Audit {
    log: AuditLog<AuditEntry>,
    dir: PathBuf,
}

/// An `AuditCursor` that is not one this server handed out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BadCursor;

impl Audit {
    /// Opens (creating, 0700/0600) `<dir>/audit-log.jsonl`, rotating past `rotate_bytes`.
    pub fn open(dir: PathBuf, rotate_bytes: u64) -> std::io::Result<Self> {
        let log = AuditLog::open(AuditOptions {
            dir: dir.clone(),
            retention: Retention::RotateAtBytes(rotate_bytes),
        })?;
        Ok(Self { log, dir })
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Writes one entry. A failed write is logged, never swallowed silently; the caller decides
    /// whether the action may still proceed.
    pub fn record(&self, e: &AuditEntry) -> std::io::Result<()> {
        self.log.append(e).inspect_err(|err| {
            tracing::error!(action = ?e.action, error = %err, "audit write failed");
        })
    }

    /// One `GetAudit` page, newest first.
    pub fn page(
        &self,
        filter: &AuditFilter,
        cursor: Option<&AuditCursor>,
    ) -> Result<(Vec<AuditEntry>, Option<AuditCursor>), PageError> {
        let cursor = cursor
            .map(decode_cursor)
            .transpose()
            .map_err(|_| PageError::BadCursor)?;
        let page = self
            .log
            .read(|e| matches(filter, e), cursor, PAGE)
            .map_err(PageError::Io)?;
        Ok((page.entries, page.next.map(encode_cursor)))
    }

    /// Every matching entry, oldest first, as JSON lines (`/api/v1/audit/export`).
    pub fn export(&self, filter: &AuditFilter) -> std::io::Result<Vec<u8>> {
        let mut out = Vec::new();
        for e in self.log.all()?.iter().filter(|e| matches(filter, e)) {
            serde_json::to_writer(&mut out, e).map_err(std::io::Error::other)?;
            out.push(b'\n');
        }
        Ok(out)
    }
}

#[derive(Debug)]
pub enum PageError {
    BadCursor,
    Io(std::io::Error),
}

fn encode_cursor(c: Cursor) -> AuditCursor {
    let json = serde_json::to_vec(&c).expect("a cursor serializes");
    AuditCursor::new(URL_SAFE_NO_PAD.encode(json)).expect("b64url is a valid cursor")
}

fn decode_cursor(c: &AuditCursor) -> Result<Cursor, BadCursor> {
    let bytes = URL_SAFE_NO_PAD.decode(c.as_str()).map_err(|_| BadCursor)?;
    serde_json::from_slice(&bytes).map_err(|_| BadCursor)
}

fn parse_time(s: &str) -> Option<chrono::DateTime<chrono::FixedOffset>> {
    chrono::DateTime::parse_from_rfc3339(s).ok()
}

/// Every set field of `f` narrows.
pub fn matches(f: &AuditFilter, e: &AuditEntry) -> bool {
    if let Some(s) = &f.stack
        && e.stack.as_deref() != Some(s.as_str())
    {
        return false;
    }
    if let Some(a) = &f.actor
        && format!("{}:{}", provider_str(e.actor.provider), e.actor.subject) != a.as_str()
    {
        return false;
    }
    if f.action.is_some_and(|a| a != e.action) {
        return false;
    }
    let at = parse_time(&e.at);
    if let Some(from) = f.from.as_ref().and_then(|t| parse_time(t.as_str()))
        && at.is_none_or(|at| at < from)
    {
        return false;
    }
    if let Some(to) = f.to.as_ref().and_then(|t| parse_time(t.as_str()))
        && at.is_none_or(|at| at > to)
    {
        return false;
    }
    true
}

pub fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

/// The audit actor for a signed-in session.
pub fn actor_of(i: &web_auth::session::Identity) -> Actor {
    let s = crate::users::SignIn::from_identity(i);
    Actor {
        provider: s.provider,
        subject: s.subject,
        username: s.username,
    }
}

/// An entry with only the fields every action has; set the rest on the result.
pub fn entry(actor: Actor, action: AuditAction, ip: IpAddr, outcome: AuditOutcome) -> AuditEntry {
    AuditEntry {
        at: now(),
        actor,
        action,
        stack: None,
        preview_id: None,
        operation_id: None,
        argv: Vec::new(),
        target: None,
        diff_summary: None,
        outcome,
        source_ip: ip.to_string(),
        network: None,
        security: None,
    }
}

/// A refusal: `denied` with the reason, and the stack it was aimed at, if any.
pub fn denied(
    actor: Actor,
    ip: IpAddr,
    stack: Option<&str>,
    reason: impl Into<String>,
) -> AuditEntry {
    let mut e = entry(
        actor,
        AuditAction::Denied,
        ip,
        AuditOutcome::Denied {
            reason: reason.into(),
        },
    );
    e.stack = stack.map(str::to_owned);
    e
}

/// Who ran a confirmed operation, on what, and the argv: everything but the outcome.
#[derive(Debug, Clone)]
pub struct OpRecord {
    pub actor: Actor,
    pub ip: IpAddr,
    pub action: AuditAction,
    pub stack: String,
    pub preview_id: PreviewId,
    pub operation_id: OperationId,
    pub argv: Vec<Vec<String>>,
    /// Saves and creates: [`crate::diff::summary`].
    pub diff_summary: Option<String>,
}

impl OpRecord {
    /// The entry, written once the operation has finished.
    pub fn finished(&self, outcome: AuditOutcome) -> AuditEntry {
        let mut e = entry(self.actor.clone(), self.action, self.ip, outcome);
        e.stack = Some(self.stack.clone());
        e.preview_id = Some(self.preview_id);
        e.operation_id = Some(self.operation_id);
        e.argv.clone_from(&self.argv);
        e.diff_summary.clone_from(&self.diff_summary);
        e
    }
}

/// A users-registry change.
pub fn user_change(actor: Actor, ip: IpAddr, added: bool, change: UserChange) -> AuditEntry {
    let action = match (added, change.after.is_some()) {
        (true, _) => AuditAction::UserAdded,
        (false, true) => AuditAction::UserChanged,
        (false, false) => AuditAction::UserRemoved,
    };
    let mut e = entry(actor, action, ip, AuditOutcome::Succeeded);
    e.target = Some(change);
    e
}

#[cfg(test)]
mod tests {
    use super::*;
    use depths_protocol::{Provider, StackName, Timestamp, UserId};

    fn actor(sub: &str) -> Actor {
        Actor {
            provider: Provider::Discord,
            subject: sub.into(),
            username: format!("u{sub}"),
        }
    }

    fn ip() -> IpAddr {
        "203.0.113.7".parse().unwrap()
    }

    fn open(d: &Path) -> Audit {
        Audit::open(d.join("audit"), 1 << 20).unwrap()
    }

    #[test]
    fn filters_narrow_by_stack_actor_action_and_time() {
        let d = tempfile::tempdir().unwrap();
        let a = open(d.path());
        let mut e1 = denied(actor("1"), ip(), Some("web"), "not_permitted");
        e1.at = "2026-10-23T10:00:00Z".into();
        let mut e2 = entry(
            actor("2"),
            AuditAction::RevealEnv,
            ip(),
            AuditOutcome::Succeeded,
        );
        e2.stack = Some("db".into());
        e2.at = "2026-10-23T12:00:00Z".into();
        a.record(&e1).unwrap();
        a.record(&e2).unwrap();
        let all = AuditFilter::default();
        assert_eq!(a.page(&all, None).unwrap().0.len(), 2);
        let f = AuditFilter {
            stack: Some(StackName::new("web").unwrap()),
            ..Default::default()
        };
        assert_eq!(a.page(&f, None).unwrap().0, vec![e1.clone()]);
        let f = AuditFilter {
            actor: Some(UserId::new("discord:2").unwrap()),
            ..Default::default()
        };
        assert_eq!(a.page(&f, None).unwrap().0, vec![e2.clone()]);
        let f = AuditFilter {
            action: Some(AuditAction::Denied),
            ..Default::default()
        };
        assert_eq!(a.page(&f, None).unwrap().0, vec![e1.clone()]);
        let f = AuditFilter {
            from: Some(Timestamp::new("2026-10-23T11:00:00Z").unwrap()),
            ..Default::default()
        };
        assert_eq!(a.page(&f, None).unwrap().0, vec![e2.clone()]);
        let f = AuditFilter {
            to: Some(Timestamp::new("2026-10-23T11:00:00+00:00").unwrap()),
            ..Default::default()
        };
        assert_eq!(a.page(&f, None).unwrap().0, vec![e1]);
    }

    #[test]
    fn pages_follow_the_cursor_and_refuse_a_forged_one() {
        let d = tempfile::tempdir().unwrap();
        let a = open(d.path());
        for i in 0..(PAGE + 5) {
            a.record(&denied(actor("1"), ip(), None, format!("n{i}")))
                .unwrap();
        }
        let (first, next) = a.page(&AuditFilter::default(), None).unwrap();
        assert_eq!(first.len(), PAGE);
        let next = next.expect("a full page has a cursor");
        let (rest, _) = a.page(&AuditFilter::default(), Some(&next)).unwrap();
        assert_eq!(rest.len(), 5);
        let forged = AuditCursor::new("AAAA").unwrap();
        assert!(matches!(
            a.page(&AuditFilter::default(), Some(&forged)),
            Err(PageError::BadCursor)
        ));
    }

    #[test]
    fn export_is_oldest_first_jsonl() {
        let d = tempfile::tempdir().unwrap();
        let a = open(d.path());
        a.record(&denied(actor("1"), ip(), None, "first")).unwrap();
        a.record(&denied(actor("1"), ip(), None, "second")).unwrap();
        let out = String::from_utf8(a.export(&AuditFilter::default()).unwrap()).unwrap();
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("first") && lines[1].contains("second"));
    }

    #[test]
    fn user_changes_name_their_action() {
        let c = |before: bool, after: bool| UserChange {
            id: "discord:2".into(),
            before: before.then(|| depths_protocol::Grant {
                role: depths_protocol::Role::Pending,
                stacks: vec![],
            }),
            after: after.then(|| depths_protocol::Grant {
                role: depths_protocol::Role::User,
                stacks: vec!["a".into()],
            }),
        };
        assert_eq!(
            user_change(actor("1"), ip(), true, c(false, true)).action,
            AuditAction::UserAdded
        );
        assert_eq!(
            user_change(actor("1"), ip(), false, c(true, true)).action,
            AuditAction::UserChanged
        );
        assert_eq!(
            user_change(actor("1"), ip(), false, c(true, false)).action,
            AuditAction::UserRemoved
        );
    }

    #[test]
    fn the_log_file_is_private() {
        use std::os::unix::fs::PermissionsExt as _;
        let d = tempfile::tempdir().unwrap();
        let a = open(d.path());
        a.record(&denied(actor("1"), ip(), None, "x")).unwrap();
        let mode = std::fs::metadata(a.dir().join(audit_log::AUDIT_FILE))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o077, 0);
    }
}
