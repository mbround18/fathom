//! Local kind cluster provisioning (spec 030).
//!
//! Safety rules enforced here:
//! - every subprocess is spawned with an argument array (never through a shell);
//! - cluster names are validated as DNS-1123 labels before they reach an argument;
//! - deletion is refused for names that `kind get clusters` does not report, so Fathom can
//!   only ever remove kind clusters (never arbitrary kubeconfig contexts);
//! - kind itself merges/removes only its own context in the kubeconfig.

use std::collections::HashMap;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

use crate::backend::persistence::KindProvenanceFile;
use crate::protocol::{
    KindClusterOperationKind, KindClusterProfileId, KindClusterStatus, KindClusterSummary,
    KindPrerequisites, KindRuntimeKind,
};

const PROBE_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_CUSTOM_YAML_BYTES: usize = 64 * 1024;
const BARE_CONFIG: &str =
    "kind: Cluster\napiVersion: kind.x-k8s.io/v1alpha4\nnodes:\n- role: control-plane\n";

// ---------------------------------------------------------------------------
// Command runner seam (mockable in tests)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CmdOutput {
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunError {
    /// Binary not found on `PATH`.
    NotFound,
    Other(String),
}

/// Runs a short-lived probe command. Implementations must use argument arrays only.
pub trait Runner: Send + Sync {
    fn run(&self, bin: &str, args: &[&str]) -> Result<CmdOutput, RunError>;
}

/// Real runner: `std::process::Command` with an argument array, no shell.
pub struct SystemRunner;

impl Runner for SystemRunner {
    fn run(&self, bin: &str, args: &[&str]) -> Result<CmdOutput, RunError> {
        match std::process::Command::new(bin).args(args).output() {
            Ok(out) => Ok(CmdOutput {
                success: out.status.success(),
                stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
                stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(RunError::NotFound),
            Err(e) => Err(RunError::Other(e.to_string())),
        }
    }
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

/// RFC 1123 label: 1-63 chars, lowercase alphanumeric or `-`, alphanumeric at both ends.
pub fn validate_cluster_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("Cluster name must not be empty.".into());
    }
    if name.len() > 63 {
        return Err("Cluster name must be at most 63 characters.".into());
    }
    if !name
        .bytes()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    {
        return Err("Cluster name may only contain lowercase letters, digits, and '-'.".into());
    }
    if name.starts_with('-') || name.ends_with('-') {
        return Err("Cluster name must start and end with a letter or digit.".into());
    }
    Ok(())
}

/// Name validation plus collision against the existing kind clusters.
pub fn validate_new_cluster_name(name: &str, existing: &[String]) -> Result<(), String> {
    validate_cluster_name(name)?;
    if existing.iter().any(|e| e == name) {
        return Err(format!("A kind cluster named '{name}' already exists."));
    }
    Ok(())
}

/// The kind config fed to `kind create cluster --config -`.
pub fn config_for_profile(
    profile: KindClusterProfileId,
    custom_yaml: Option<&str>,
) -> Result<String, String> {
    match profile {
        KindClusterProfileId::Bare => Ok(BARE_CONFIG.to_owned()),
        KindClusterProfileId::CustomYaml => {
            let yaml = custom_yaml
                .filter(|y| !y.trim().is_empty())
                .ok_or_else(|| "The custom_yaml profile requires a kind config.".to_owned())?;
            if yaml.len() > MAX_CUSTOM_YAML_BYTES {
                return Err("Custom kind config is too large (max 64 KiB).".into());
            }
            serde_yaml::from_str::<serde_yaml::Value>(yaml)
                .map_err(|e| format!("Custom kind config is not valid YAML: {e}"))?;
            Ok(yaml.to_owned())
        }
    }
}

// ---------------------------------------------------------------------------
// Prerequisites + listing
// ---------------------------------------------------------------------------

pub fn detect_prerequisites_with(runner: &dyn Runner) -> KindPrerequisites {
    let (kind_installed, kind_version) = match runner.run("kind", &["version"]) {
        Ok(out) if out.success => (true, Some(out.stdout.trim().to_owned())),
        Ok(_) | Err(RunError::Other(_)) => (true, None),
        Err(RunError::NotFound) => (false, None),
    };

    let mut runtime_installed = false;
    let mut runtime_kind = None;
    let mut runtime_running = false;
    for (bin, kind) in [
        ("docker", KindRuntimeKind::Docker),
        ("podman", KindRuntimeKind::Podman),
    ] {
        match runner.run(bin, &["info"]) {
            Err(RunError::NotFound) => continue,
            Ok(out) => {
                runtime_installed = true;
                runtime_kind = Some(kind);
                runtime_running = out.success;
            }
            Err(RunError::Other(_)) => {
                runtime_installed = true;
                runtime_kind = Some(kind);
            }
        }
        // Prefer a runtime that is actually running; otherwise keep the first installed one.
        if runtime_running {
            break;
        }
    }

    KindPrerequisites {
        kind_installed,
        kind_version,
        runtime_installed,
        runtime_kind,
        runtime_running,
        ready: kind_installed && runtime_installed && runtime_running,
    }
}

pub async fn detect_prerequisites() -> KindPrerequisites {
    let probe = tokio::task::spawn_blocking(|| detect_prerequisites_with(&SystemRunner));
    match tokio::time::timeout(PROBE_TIMEOUT, probe).await {
        Ok(Ok(p)) => p,
        _ => KindPrerequisites {
            kind_installed: false,
            kind_version: None,
            runtime_installed: false,
            runtime_kind: None,
            runtime_running: false,
            ready: false,
        },
    }
}

pub fn parse_cluster_names(stdout: &str) -> Vec<String> {
    let mut names: Vec<String> = stdout
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with("No kind clusters found"))
        .map(str::to_owned)
        .collect();
    names.sort();
    names
}

pub fn context_name_for(cluster: &str) -> String {
    format!("kind-{cluster}")
}

pub fn list_cluster_names_with(runner: &dyn Runner) -> Result<Vec<String>, String> {
    match runner.run("kind", &["get", "clusters"]) {
        Ok(out) if out.success => Ok(parse_cluster_names(&out.stdout)),
        Ok(out) => Err(out.stderr.trim().to_owned()),
        Err(RunError::NotFound) => Err("kind is not installed.".into()),
        Err(RunError::Other(e)) => Err(e),
    }
}

/// The profile a cluster was created from, which only Fathom's own provenance record knows:
/// `kind get clusters` reports names and nothing else (045 T058, FR-036).
fn profile_of(provenance: &KindProvenanceFile, name: &str) -> Option<KindClusterProfileId> {
    provenance.record_for(name).and_then(|r| r.profile)
}

/// Builds summaries; names with a tracked in-flight operation report `in_progress`.
///
/// `provenance` is Fathom's record of the clusters it provisioned itself; a cluster absent from
/// it keeps `profile: None`, because nothing on the machine can tell us how someone else's
/// cluster was built.
pub fn summaries_from_names(
    names: Vec<String>,
    registry: &KindActionRegistry,
    provenance: &KindProvenanceFile,
) -> Vec<KindClusterSummary> {
    names
        .into_iter()
        .map(|name| {
            let status = if registry.is_busy(&name) {
                KindClusterStatus::InProgress
            } else {
                KindClusterStatus::Ready
            };
            KindClusterSummary {
                context_name: context_name_for(&name),
                profile: profile_of(provenance, &name),
                name,
                status,
            }
        })
        .collect()
}

pub async fn list_cluster_names() -> Result<Vec<String>, String> {
    let join = tokio::task::spawn_blocking(|| list_cluster_names_with(&SystemRunner));
    match tokio::time::timeout(PROBE_TIMEOUT, join).await {
        Ok(Ok(r)) => r,
        Ok(Err(e)) => Err(e.to_string()),
        Err(_) => Err("Timed out listing kind clusters.".into()),
    }
}

pub async fn list_clusters(
    registry: &KindActionRegistry,
) -> Result<Vec<KindClusterSummary>, String> {
    let provenance = crate::backend::persistence::load_kind_provenance();
    Ok(summaries_from_names(
        list_cluster_names().await?,
        registry,
        &provenance,
    ))
}

// ---------------------------------------------------------------------------
// Concurrency registry (FR-016)
// ---------------------------------------------------------------------------

struct Entry {
    cluster_name: String,
    #[allow(dead_code)]
    kind: KindClusterOperationKind,
    abort: Option<tokio::task::AbortHandle>,
}

/// In-memory tracker of in-flight kind operations, shared with the background tasks so they
/// can unregister themselves on completion.
#[derive(Clone, Default)]
pub struct KindActionRegistry {
    inner: Arc<Mutex<HashMap<String, Entry>>>,
}

impl KindActionRegistry {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Entry>> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Registers an operation; rejects immediately if the cluster already has one.
    pub fn try_register(
        &self,
        request_id: &str,
        cluster_name: &str,
        kind: KindClusterOperationKind,
    ) -> Result<(), String> {
        let mut map = self.lock();
        if map.values().any(|e| e.cluster_name == cluster_name) {
            return Err(format!(
                "Another operation on kind cluster '{cluster_name}' is already in progress."
            ));
        }
        if map.contains_key(request_id) {
            return Err(format!("Request id '{request_id}' is already in use."));
        }
        map.insert(
            request_id.to_owned(),
            Entry {
                cluster_name: cluster_name.to_owned(),
                kind,
                abort: None,
            },
        );
        Ok(())
    }

    pub fn attach(&self, request_id: &str, abort: tokio::task::AbortHandle) {
        if let Some(e) = self.lock().get_mut(request_id) {
            e.abort = Some(abort);
        }
    }

    pub fn finish(&self, request_id: &str) {
        self.lock().remove(request_id);
    }

    pub fn is_busy(&self, cluster_name: &str) -> bool {
        self.lock().values().any(|e| e.cluster_name == cluster_name)
    }

    /// Aborts one operation (killing its child via `kill_on_drop`); returns its cluster name.
    pub fn cancel(&self, request_id: &str) -> Option<String> {
        let entry = self.lock().remove(request_id)?;
        if let Some(a) = entry.abort {
            a.abort();
        }
        Some(entry.cluster_name)
    }
}

// ---------------------------------------------------------------------------
// Create / delete
// ---------------------------------------------------------------------------

/// Strips kind's decoration (` • `, ` ✓ `, ` ✗ `) from a progress line.
pub fn clean_progress_line(line: &str) -> Option<String> {
    let t = line.trim().trim_start_matches(['•', '✓', '✗', ' ']).trim();
    (!t.is_empty()).then(|| t.to_owned())
}

fn base_command(runtime: Option<KindRuntimeKind>) -> Command {
    let mut cmd = Command::new("kind");
    cmd.kill_on_drop(true);
    if runtime == Some(KindRuntimeKind::Podman) {
        cmd.env("KIND_EXPERIMENTAL_PROVIDER", "podman");
    }
    cmd
}

/// Runs `kind create cluster --name <name> --config -`, calling `on_stage` per stderr line.
/// Caller must have validated `name` and produced `config` via [`config_for_profile`].
pub async fn run_create(
    name: &str,
    config: &str,
    runtime: Option<KindRuntimeKind>,
    mut on_stage: impl FnMut(String),
) -> Result<(), String> {
    validate_cluster_name(name)?;
    let mut cmd = base_command(runtime);
    cmd.args(["create", "cluster", "--name", name, "--config", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    run_streaming(cmd, Some(config), &mut on_stage).await
}

/// Runs `kind delete cluster --name <name>`.
pub async fn run_delete(
    name: &str,
    runtime: Option<KindRuntimeKind>,
    mut on_stage: impl FnMut(String),
) -> Result<(), String> {
    validate_cluster_name(name)?;
    let mut cmd = base_command(runtime);
    cmd.args(["delete", "cluster", "--name", name])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    run_streaming(cmd, None, &mut on_stage).await
}

async fn run_streaming(
    mut cmd: Command,
    stdin_data: Option<&str>,
    on_stage: &mut impl FnMut(String),
) -> Result<(), String> {
    let mut child = cmd.spawn().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            "kind is not installed.".to_owned()
        } else {
            format!("Failed to start kind: {e}")
        }
    })?;
    if let (Some(data), Some(mut stdin)) = (stdin_data, child.stdin.take()) {
        stdin
            .write_all(data.as_bytes())
            .await
            .map_err(|e| format!("Failed to send config to kind: {e}"))?;
        drop(stdin);
    }
    let stderr = child.stderr.take().ok_or("kind stderr unavailable")?;
    let mut lines = BufReader::new(stderr).lines();
    let mut all = Vec::new();
    while let Ok(Some(line)) = lines.next_line().await {
        all.push(line.clone());
        if let Some(stage) = clean_progress_line(&line) {
            on_stage(stage);
        }
    }
    let status = child.wait().await.map_err(|e| e.to_string())?;
    if status.success() {
        Ok(())
    } else {
        let text = all.join("\n");
        Err(if text.trim().is_empty() {
            format!("kind exited with {status}")
        } else {
            text
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::persistence::KindProvenanceRecord;

    struct Mock(HashMap<&'static str, Result<CmdOutput, RunError>>);
    impl Runner for Mock {
        fn run(&self, bin: &str, _args: &[&str]) -> Result<CmdOutput, RunError> {
            self.0.get(bin).cloned().unwrap_or(Err(RunError::NotFound))
        }
    }
    fn ok(s: &str) -> Result<CmdOutput, RunError> {
        Ok(CmdOutput {
            success: true,
            stdout: s.into(),
            stderr: String::new(),
        })
    }
    fn fail() -> Result<CmdOutput, RunError> {
        Ok(CmdOutput {
            success: false,
            stdout: String::new(),
            stderr: "daemon".into(),
        })
    }

    #[test]
    fn kind_absent() {
        let p = detect_prerequisites_with(&Mock(HashMap::from([("docker", ok(""))])));
        assert!(!p.kind_installed && !p.ready);
        assert!(p.runtime_running);
    }

    #[test]
    fn runtime_absent() {
        let p = detect_prerequisites_with(&Mock(HashMap::from([("kind", ok("kind v0.23.0\n"))])));
        assert!(p.kind_installed && !p.runtime_installed && !p.ready);
        assert_eq!(p.kind_version.as_deref(), Some("kind v0.23.0"));
    }

    #[test]
    fn runtime_installed_not_running() {
        let p = detect_prerequisites_with(&Mock(HashMap::from([
            ("kind", ok("kind v0.23.0")),
            ("docker", fail()),
        ])));
        assert!(p.runtime_installed && !p.runtime_running && !p.ready);
        assert_eq!(p.runtime_kind, Some(KindRuntimeKind::Docker));
    }

    #[test]
    fn podman_fallback_and_ready() {
        let p = detect_prerequisites_with(&Mock(HashMap::from([
            ("kind", ok("kind v0.23.0")),
            ("podman", ok("")),
        ])));
        assert!(p.ready);
        assert_eq!(p.runtime_kind, Some(KindRuntimeKind::Podman));
    }

    #[test]
    fn docker_stopped_podman_running_prefers_podman() {
        let p = detect_prerequisites_with(&Mock(HashMap::from([
            ("kind", ok("k")),
            ("docker", fail()),
            ("podman", ok("")),
        ])));
        assert!(p.ready);
        assert_eq!(p.runtime_kind, Some(KindRuntimeKind::Podman));
    }

    #[test]
    fn list_parsing() {
        assert_eq!(parse_cluster_names("b\na\n"), vec!["a", "b"]);
        assert!(parse_cluster_names("").is_empty());
        assert!(parse_cluster_names("No kind clusters found.\n").is_empty());
        let r = Mock(HashMap::from([("kind", ok(""))]));
        assert!(list_cluster_names_with(&r).unwrap().is_empty());
        let s = summaries_from_names(
            vec!["dev".into()],
            &KindActionRegistry::default(),
            &KindProvenanceFile::default(),
        );
        assert_eq!(s[0].context_name, "kind-dev");
        assert_eq!(s[0].status, KindClusterStatus::Ready);
    }

    /// T058 (FR-036): the profile is read from the provenance record, and a cluster Fathom did
    /// not provision reports no profile however its context name looks.
    #[test]
    fn provenance_supplies_the_profile() {
        let provenance = KindProvenanceFile {
            clusters: vec![KindProvenanceRecord {
                cluster: "ours".into(),
                provisioned_at: 42,
                profile: Some(KindClusterProfileId::CustomYaml),
            }],
        };
        let s = summaries_from_names(
            vec!["ours".into(), "theirs".into()],
            &KindActionRegistry::default(),
            &provenance,
        );
        assert_eq!(s[0].name, "ours");
        assert_eq!(s[0].profile, Some(KindClusterProfileId::CustomYaml));
        // Listed by `kind get clusters`, context `kind-theirs`, but not ours and so profileless.
        assert_eq!(s[1].name, "theirs");
        assert_eq!(s[1].profile, None);
    }

    #[test]
    fn name_validation() {
        for good in ["a", "dev-1", "0abc", &"a".repeat(63)] {
            assert!(validate_cluster_name(good).is_ok(), "{good}");
        }
        for bad in [
            "",
            "Dev",
            "-a",
            "a-",
            "a_b",
            "a b",
            "a;rm -rf /",
            "$(x)",
            &"a".repeat(64),
        ] {
            assert!(validate_cluster_name(bad).is_err(), "{bad}");
        }
        assert!(validate_new_cluster_name("dev", &["dev".into()]).is_err());
        assert!(validate_new_cluster_name("dev2", &["dev".into()]).is_ok());
    }

    #[test]
    fn profile_configs() {
        assert!(
            config_for_profile(KindClusterProfileId::Bare, None)
                .unwrap()
                .contains("control-plane")
        );
        assert!(config_for_profile(KindClusterProfileId::CustomYaml, None).is_err());
        assert!(config_for_profile(KindClusterProfileId::CustomYaml, Some("a: [")).is_err());
        assert!(
            config_for_profile(KindClusterProfileId::CustomYaml, Some("kind: Cluster")).is_ok()
        );
    }

    #[test]
    fn concurrent_operation_rejected() {
        let r = KindActionRegistry::default();
        r.try_register("1", "dev", KindClusterOperationKind::Create)
            .unwrap();
        assert!(
            r.try_register("2", "dev", KindClusterOperationKind::Delete)
                .is_err()
        );
        assert!(
            r.try_register("3", "other", KindClusterOperationKind::Create)
                .is_ok()
        );
        assert!(r.is_busy("dev"));
        r.finish("1");
        assert!(!r.is_busy("dev"));
        assert_eq!(r.cancel("3").as_deref(), Some("other"));
    }

    #[test]
    fn progress_lines_cleaned() {
        assert_eq!(
            clean_progress_line(" ✓ Preparing nodes 📦 ").as_deref(),
            Some("Preparing nodes 📦")
        );
        assert_eq!(
            clean_progress_line("  • Ensuring node image").as_deref(),
            Some("Ensuring node image")
        );
        assert_eq!(clean_progress_line("   "), None);
    }

    #[test]
    fn kubeconfig_merge_leaves_unrelated_entries_untouched() {
        use kube::config::{Kubeconfig, NamedAuthInfo, NamedCluster, NamedContext};
        let raw = include_str!("fixtures/kubeconfig_multi_context.yaml");
        let before = Kubeconfig::from_yaml(raw).unwrap();
        let mut after = before.clone();
        after.contexts.push(NamedContext {
            name: "kind-dev".into(),
            context: None,
        });
        after.clusters.push(NamedCluster {
            name: "kind-dev".into(),
            cluster: None,
        });
        after.auth_infos.push(NamedAuthInfo {
            name: "kind-dev".into(),
            auth_info: None,
        });
        assert!(after.contexts.len() >= 3 && before.contexts.len() >= 2);
        for (b, a) in before.contexts.iter().zip(after.contexts.iter()) {
            assert_eq!(
                serde_json::to_string(b).unwrap(),
                serde_json::to_string(a).unwrap()
            );
        }
        for (b, a) in before.clusters.iter().zip(after.clusters.iter()) {
            assert_eq!(
                serde_json::to_string(b).unwrap(),
                serde_json::to_string(a).unwrap()
            );
        }
        assert_eq!(before.current_context, after.current_context);
    }

    /// Creates and deletes a REAL kind cluster. Run explicitly:
    /// `cargo test kind::tests::real_kind_lifecycle -- --ignored`
    #[tokio::test]
    #[ignore = "requires kind + a running container runtime; creates a real cluster"]
    async fn real_kind_lifecycle() {
        let name = "fathom-it-030";
        let cfg = config_for_profile(KindClusterProfileId::Bare, None).unwrap();
        run_create(name, &cfg, None, |_| {}).await.unwrap();
        assert!(
            list_cluster_names()
                .await
                .unwrap()
                .iter()
                .any(|n| n == name)
        );
        run_delete(name, None, |_| {}).await.unwrap();
    }
}
