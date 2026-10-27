//! The `keel_events` line format (research R3) and its parser.
//!
//! The `keel_events` Ansible callback (apps/keel/runner/ansible/callback/keel_events.py) writes
//! one JSON object per line to the fd in `KEEL_EVENT_FD`. Every line carries `"v": 1` and an
//! `"event"` kind:
//!
//! | `event` | other fields |
//! |---|---|
//! | `playbook_start` | `playbook` |
//! | `play_start` | `play` |
//! | `task_start` | `task`, `task_id`, `action`, `handler` |
//! | `ok` / `changed` / `failed` / `unreachable` / `skipped` | `host`, `task`, `task_id`, `action`, `ignore_errors`, `result` |
//! | `stats` | `stats`: `{host: {ok, changed, failures, unreachable, skipped, rescued, ignored}}` |
//!
//! A `no_log` result is written as `{"censored": "…"}` only (plus `changed`); the parser keeps
//! that guarantee even against a callback that did not.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The only line version this parser accepts.
pub const LINE_VERSION: u64 = 1;

/// Ansible's per-host summary (`AggregateStats.summarize`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostStats {
    pub ok: u64,
    pub changed: u64,
    pub failures: u64,
    pub unreachable: u64,
    pub skipped: u64,
    pub rescued: u64,
    pub ignored: u64,
}

/// The outcome of one task on one host.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostStatus {
    Ok,
    Changed,
    Failed,
    Unreachable,
    Skipped,
}

/// One parsed `keel_events` line.
#[derive(Debug, Clone, PartialEq)]
pub enum KeelEvent {
    PlaybookStart { playbook: String },
    PlayStart { play: String },
    TaskStart { task: String, task_id: String, action: String, handler: bool },
    HostResult(HostResult),
    Stats { stats: BTreeMap<String, HostStats> },
}

/// A per-host task result.
#[derive(Debug, Clone, PartialEq)]
pub struct HostResult {
    pub status: HostStatus,
    pub host: String,
    pub task: String,
    pub task_id: String,
    pub action: String,
    pub ignore_errors: bool,
    /// The module result, already censored when the task was `no_log`.
    pub result: Value,
}

/// What `run_host_results.first_failure` holds (data-model.md).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FirstFailure {
    pub task: String,
    pub module: String,
    pub message: String,
    pub rc: Option<i64>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum EventError {
    #[error("not a JSON object: {0}")]
    NotJson(String),
    #[error("unsupported line version {0:?}")]
    Version(Option<u64>),
    #[error("unknown event kind {0:?}")]
    UnknownKind(String),
    #[error("field `{0}` missing or of the wrong type")]
    Field(&'static str),
}

impl KeelEvent {
    /// The `event` kind, as on the line.
    pub fn kind(&self) -> &'static str {
        match self {
            KeelEvent::PlaybookStart { .. } => "playbook_start",
            KeelEvent::PlayStart { .. } => "play_start",
            KeelEvent::TaskStart { .. } => "task_start",
            KeelEvent::HostResult(r) => r.status.as_str(),
            KeelEvent::Stats { .. } => "stats",
        }
    }

    /// The host a result is about, for `run_events.host`.
    pub fn host(&self) -> Option<&str> {
        match self {
            KeelEvent::HostResult(r) => Some(&r.host),
            _ => None,
        }
    }

    /// The task name, for `run_events.task`.
    pub fn task(&self) -> Option<&str> {
        match self {
            KeelEvent::TaskStart { task, .. } => Some(task),
            KeelEvent::HostResult(r) => Some(&r.task),
            _ => None,
        }
    }
}

impl HostStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            HostStatus::Ok => "ok",
            HostStatus::Changed => "changed",
            HostStatus::Failed => "failed",
            HostStatus::Unreachable => "unreachable",
            HostStatus::Skipped => "skipped",
        }
    }

    fn from_kind(kind: &str) -> Option<Self> {
        Some(match kind {
            "ok" => HostStatus::Ok,
            "changed" => HostStatus::Changed,
            "failed" => HostStatus::Failed,
            "unreachable" => HostStatus::Unreachable,
            "skipped" => HostStatus::Skipped,
            _ => return None,
        })
    }
}

impl HostResult {
    /// Whether the result was hidden by `no_log`.
    pub fn is_censored(&self) -> bool {
        self.result.get("censored").is_some()
    }

    /// The actionable context for a failure that counts (not `ignore_errors`): task, module,
    /// message and return code. `None` for anything else.
    pub fn first_failure(&self) -> Option<FirstFailure> {
        let counts = match self.status {
            HostStatus::Failed => !self.ignore_errors,
            HostStatus::Unreachable => true,
            _ => false,
        };
        if !counts {
            return None;
        }
        let message = if self.is_censored() {
            "the output has been hidden (no_log)".to_string()
        } else {
            self.result
                .get("msg")
                .and_then(Value::as_str)
                .or_else(|| self.result.get("stderr").and_then(Value::as_str))
                .unwrap_or("")
                .to_string()
        };
        Some(FirstFailure {
            task: self.task.clone(),
            module: self.action.clone(),
            message,
            rc: self.result.get("rc").and_then(Value::as_i64),
        })
    }
}

fn str_field(obj: &serde_json::Map<String, Value>, name: &'static str) -> Result<String, EventError> {
    obj.get(name).and_then(Value::as_str).map(str::to_owned).ok_or(EventError::Field(name))
}

fn bool_field(obj: &serde_json::Map<String, Value>, name: &'static str) -> Result<bool, EventError> {
    match obj.get(name) {
        None => Ok(false),
        Some(Value::Bool(b)) => Ok(*b),
        Some(_) => Err(EventError::Field(name)),
    }
}

/// Keep only what a censored result may show, whatever the callback wrote.
fn enforce_censoring(result: Value) -> Value {
    let censored = match &result {
        Value::Object(o) => o.get("censored").cloned().or_else(|| {
            matches!(o.get("_ansible_no_log"), Some(Value::Bool(true)))
                .then(|| Value::String("the output has been hidden due to no_log".into()))
        }),
        _ => None,
    };
    match censored {
        None => result,
        Some(c) => {
            let mut out = serde_json::Map::new();
            out.insert("censored".into(), c);
            if let Some(changed @ Value::Bool(_)) = result.get("changed") {
                out.insert("changed".into(), changed.clone());
            }
            Value::Object(out)
        }
    }
}

/// Parse one `keel_events` line (trailing newline allowed).
pub fn parse_line(line: &str) -> Result<KeelEvent, EventError> {
    let value: Value = serde_json::from_str(line.trim_end_matches(['\n', '\r']))
        .map_err(|e| EventError::NotJson(e.to_string()))?;
    let Value::Object(obj) = value else {
        return Err(EventError::NotJson("not an object".into()));
    };
    let version = obj.get("v").and_then(Value::as_u64);
    if version != Some(LINE_VERSION) {
        return Err(EventError::Version(version));
    }
    let kind = str_field(&obj, "event")?;
    match kind.as_str() {
        "playbook_start" => Ok(KeelEvent::PlaybookStart { playbook: str_field(&obj, "playbook")? }),
        "play_start" => Ok(KeelEvent::PlayStart { play: str_field(&obj, "play")? }),
        "task_start" => Ok(KeelEvent::TaskStart {
            task: str_field(&obj, "task")?,
            task_id: str_field(&obj, "task_id")?,
            action: str_field(&obj, "action")?,
            handler: bool_field(&obj, "handler")?,
        }),
        "stats" => {
            let stats = obj.get("stats").cloned().ok_or(EventError::Field("stats"))?;
            let stats = serde_json::from_value(stats).map_err(|_| EventError::Field("stats"))?;
            Ok(KeelEvent::Stats { stats })
        }
        other => {
            let status = HostStatus::from_kind(other).ok_or_else(|| EventError::UnknownKind(other.into()))?;
            let result = match obj.get("result") {
                Some(v @ Value::Object(_)) => enforce_censoring(v.clone()),
                None | Some(Value::Null) => Value::Object(Default::default()),
                Some(_) => return Err(EventError::Field("result")),
            };
            Ok(KeelEvent::HostResult(HostResult {
                status,
                host: str_field(&obj, "host")?,
                task: str_field(&obj, "task")?,
                task_id: str_field(&obj, "task_id")?,
                action: str_field(&obj, "action")?,
                ignore_errors: bool_field(&obj, "ignore_errors")?,
                result,
            }))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Lines as the keel_events callback writes them (recorded from its unit-test fixtures).
    const PLAYBOOK_START: &str = r#"{"v": 1, "event": "playbook_start", "playbook": "site.yml"}"#;
    const PLAY_START: &str = r#"{"v": 1, "event": "play_start", "play": "Base hosts"}"#;
    const TASK_START: &str = r#"{"v": 1, "event": "task_start", "task": "Install packages", "task_id": "0242ac11-0002-0001-0000-000000000010", "action": "ansible.builtin.apt", "handler": false}"#;
    const OK: &str = r#"{"v": 1, "event": "ok", "host": "web-1", "task": "Install packages", "task_id": "0242ac11-0002-0001-0000-000000000010", "action": "ansible.builtin.apt", "ignore_errors": false, "result": {"changed": false, "cache_updated": false}}"#;
    const CHANGED: &str = r#"{"v": 1, "event": "changed", "host": "web-1", "task": "Write config", "task_id": "t2", "action": "ansible.builtin.template", "ignore_errors": false, "result": {"changed": true, "dest": "/etc/app.conf"}}"#;
    const FAILED: &str = r#"{"v": 1, "event": "failed", "host": "web-1", "task": "Restart app", "task_id": "t3", "action": "ansible.builtin.command", "ignore_errors": false, "result": {"changed": true, "rc": 3, "msg": "non-zero return code", "stderr": "unit app.service not found"}}"#;
    const FAILED_IGNORED: &str = r#"{"v": 1, "event": "failed", "host": "web-1", "task": "Probe", "task_id": "t4", "action": "ansible.builtin.command", "ignore_errors": true, "result": {"rc": 1, "msg": "probe failed"}}"#;
    const UNREACHABLE: &str = r#"{"v": 1, "event": "unreachable", "host": "db-1", "task": "Gathering Facts", "task_id": "t0", "action": "ansible.builtin.gather_facts", "ignore_errors": false, "result": {"unreachable": true, "msg": "Failed to connect to the host via ssh"}}"#;
    const SKIPPED: &str = r#"{"v": 1, "event": "skipped", "host": "web-1", "task": "Only on Debian 12", "task_id": "t5", "action": "ansible.builtin.debug", "ignore_errors": false, "result": {"skip_reason": "Conditional result was False"}}"#;
    const CENSORED: &str = r#"{"v": 1, "event": "changed", "host": "web-1", "task": "Set password", "task_id": "t6", "action": "ansible.builtin.user", "ignore_errors": false, "result": {"censored": "the output has been hidden due to the fact that 'no_log: true' was specified for this result", "changed": true}}"#;
    const STATS: &str = r#"{"v": 1, "event": "stats", "stats": {"web-1": {"ok": 4, "changed": 2, "failures": 1, "unreachable": 0, "skipped": 1, "rescued": 0, "ignored": 1}, "db-1": {"ok": 0, "changed": 0, "failures": 0, "unreachable": 1, "skipped": 0, "rescued": 0, "ignored": 0}}}"#;

    #[test]
    fn playbook_play_and_task_starts() {
        assert_eq!(parse_line(PLAYBOOK_START).unwrap(), KeelEvent::PlaybookStart { playbook: "site.yml".into() });
        assert_eq!(parse_line(PLAY_START).unwrap(), KeelEvent::PlayStart { play: "Base hosts".into() });
        let KeelEvent::TaskStart { task, task_id, action, handler } = parse_line(TASK_START).unwrap() else {
            panic!("task_start")
        };
        assert_eq!(task, "Install packages");
        assert_eq!(task_id, "0242ac11-0002-0001-0000-000000000010");
        assert_eq!(action, "ansible.builtin.apt");
        assert!(!handler);
    }

    #[test]
    fn per_host_results() {
        for (line, status, host) in [
            (OK, HostStatus::Ok, "web-1"),
            (CHANGED, HostStatus::Changed, "web-1"),
            (FAILED, HostStatus::Failed, "web-1"),
            (UNREACHABLE, HostStatus::Unreachable, "db-1"),
            (SKIPPED, HostStatus::Skipped, "web-1"),
        ] {
            let ev = parse_line(&format!("{line}\n")).unwrap();
            assert_eq!(ev.kind(), status.as_str());
            assert_eq!(ev.host(), Some(host));
            let KeelEvent::HostResult(r) = ev else { panic!("host result") };
            assert_eq!(r.status, status);
        }
    }

    #[test]
    fn first_failure_carries_task_module_message_and_rc() {
        let KeelEvent::HostResult(r) = parse_line(FAILED).unwrap() else { panic!() };
        assert_eq!(
            r.first_failure(),
            Some(FirstFailure {
                task: "Restart app".into(),
                module: "ansible.builtin.command".into(),
                message: "non-zero return code".into(),
                rc: Some(3),
            })
        );
        let KeelEvent::HostResult(r) = parse_line(UNREACHABLE).unwrap() else { panic!() };
        assert_eq!(r.first_failure().unwrap().message, "Failed to connect to the host via ssh");
        let KeelEvent::HostResult(r) = parse_line(FAILED_IGNORED).unwrap() else { panic!() };
        assert!(r.ignore_errors);
        assert_eq!(r.first_failure(), None, "ignored failures are not the first failure");
        let KeelEvent::HostResult(r) = parse_line(OK).unwrap() else { panic!() };
        assert_eq!(r.first_failure(), None);
    }

    #[test]
    fn stats_summary() {
        let KeelEvent::Stats { stats } = parse_line(STATS).unwrap() else { panic!("stats") };
        assert_eq!(stats.len(), 2);
        assert_eq!(
            stats["web-1"],
            HostStats { ok: 4, changed: 2, failures: 1, unreachable: 0, skipped: 1, rescued: 0, ignored: 1 }
        );
        assert_eq!(stats["db-1"].unreachable, 1);
    }

    #[test]
    fn censored_results_stay_censored() {
        let KeelEvent::HostResult(r) = parse_line(CENSORED).unwrap() else { panic!() };
        assert!(r.is_censored());
        assert_eq!(r.result.as_object().unwrap().len(), 2);

        // A callback that leaked fields next to `censored`, or only flagged `_ansible_no_log`,
        // still yields a censored result.
        let leaky = r#"{"v": 1, "event": "ok", "host": "h", "task": "t", "task_id": "i", "action": "a", "result": {"censored": "hidden", "password": "hunter2", "changed": false}}"#;
        let KeelEvent::HostResult(r) = parse_line(leaky).unwrap() else { panic!() };
        assert!(!r.result.to_string().contains("hunter2"));
        let flagged = r#"{"v": 1, "event": "failed", "host": "h", "task": "t", "task_id": "i", "action": "a", "result": {"_ansible_no_log": true, "msg": "token=hunter2"}}"#;
        let KeelEvent::HostResult(r) = parse_line(flagged).unwrap() else { panic!() };
        assert!(r.is_censored());
        assert!(!r.result.to_string().contains("hunter2"));
        assert!(!r.first_failure().unwrap().message.contains("hunter2"));
    }

    #[test]
    fn malformed_lines_are_refused() {
        assert!(matches!(parse_line("PLAY [all] ****"), Err(EventError::NotJson(_))));
        assert!(matches!(parse_line("[1,2]"), Err(EventError::NotJson(_))));
        assert_eq!(parse_line(r#"{"event": "ok"}"#), Err(EventError::Version(None)));
        assert_eq!(parse_line(r#"{"v": 2, "event": "ok"}"#), Err(EventError::Version(Some(2))));
        assert_eq!(
            parse_line(r#"{"v": 1, "event": "runner_on_async_poll"}"#),
            Err(EventError::UnknownKind("runner_on_async_poll".into()))
        );
        assert_eq!(parse_line(r#"{"v": 1, "event": "ok", "task": "t"}"#), Err(EventError::Field("host")));
        assert_eq!(
            parse_line(r#"{"v": 1, "event": "stats", "stats": {"h": {"ok": "many"}}}"#),
            Err(EventError::Field("stats"))
        );
        assert_eq!(
            parse_line(r#"{"v": 1, "event": "ok", "host": "h", "task": "t", "task_id": "i", "action": "a", "result": "x"}"#),
            Err(EventError::Field("result"))
        );
    }
}
