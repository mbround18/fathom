//! The exec-plugin runner (FR-003, FR-007, FR-009). argv only (never a shell), no TTY, bounded
//! by a timeout and output caps, killed on drop, no console window on Windows.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use chrono::{DateTime, Utc};
use kube::config::{ExecConfig, ExecInteractiveMode};
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use tokio::io::{AsyncRead, AsyncReadExt};

use super::detect::{self, Resolution};
use super::redact;
use crate::protocol::{
    CloudProvider, ExecPolicy, PLUGIN_ERROR_MAX_MESSAGE_BYTES, PluginError, PluginErrorKind,
};

pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(20);
pub const STDOUT_CAP: usize = 64 * 1024;
pub const STDERR_CAP: usize = 4 * 1024;
const DEFAULT_API_VERSION: &str = "client.authentication.k8s.io/v1beta1";

/// One plugin invocation as described by a kubeconfig `exec` entry.
#[derive(Debug, Clone)]
pub struct ExecSpec {
    pub command: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub api_version: String,
    pub interactive_always: bool,
    pub provide_cluster_info: bool,
    /// `spec.cluster` for `KUBERNETES_EXEC_INFO`, only when `provide_cluster_info`.
    pub cluster_info: Option<serde_json::Value>,
}

/// Kubeconfig `exec.env` names that `allowKnownOnly` still passes to the plugin: they select a
/// profile, region or config file for the known plugins and cannot change what code the child
/// loads. Everything else (`LD_PRELOAD`, `PATH`, `AWS_CONTAINER_CREDENTIALS_FULL_URI`, ...) is
/// dropped under that policy, because a shared kubeconfig controls these values.
pub const KNOWN_ONLY_ENV_ALLOWLIST: &[&str] = &[
    "AWS_PROFILE",
    "AWS_REGION",
    "AWS_DEFAULT_REGION",
    "AWS_CONFIG_FILE",
    "AWS_SHARED_CREDENTIALS_FILE",
    "CLOUDSDK_CORE_PROJECT",
    "CLOUDSDK_CONFIG",
    "AZURE_CONFIG_DIR",
];

impl ExecSpec {
    /// The spec as `policy` lets it run: `allowAll` passes the kubeconfig `env` through verbatim,
    /// `allowKnownOnly` keeps only [`KNOWN_ONLY_ENV_ALLOWLIST`].
    pub fn restricted_to(mut self, policy: ExecPolicy) -> Self {
        if policy == ExecPolicy::AllowKnownOnly {
            self.env
                .retain(|(k, _)| KNOWN_ONLY_ENV_ALLOWLIST.contains(&k.as_str()));
        }
        self
    }

    pub fn from_kube(exec: &ExecConfig) -> Option<Self> {
        let command = exec.command.clone().filter(|c| !c.trim().is_empty())?;
        let env = exec
            .env
            .iter()
            .flatten()
            .filter_map(|m| Some((m.get("name")?.clone(), m.get("value")?.clone())))
            .collect();
        let cluster_info = if exec.provide_cluster_info {
            exec.cluster
                .as_ref()
                .and_then(|c| serde_json::to_value(c).ok())
        } else {
            None
        };
        Some(Self {
            command,
            args: exec.args.clone().unwrap_or_default(),
            env,
            api_version: exec
                .api_version
                .clone()
                .unwrap_or_else(|| DEFAULT_API_VERSION.to_owned()),
            interactive_always: exec.interactive_mode == Some(ExecInteractiveMode::Always),
            provide_cluster_info: exec.provide_cluster_info,
            cluster_info,
        })
    }

    /// Stable identity of this invocation: command, args, env, apiVersion, and cluster info
    /// when provided, so distinct clusters never share a token (FR-005).
    pub fn identity(&self) -> u64 {
        let mut h = DefaultHasher::new();
        self.command.hash(&mut h);
        self.args.hash(&mut h);
        self.env.hash(&mut h);
        self.api_version.hash(&mut h);
        if self.provide_cluster_info {
            self.cluster_info
                .as_ref()
                .map(ToString::to_string)
                .hash(&mut h);
        }
        h.finish()
    }

    fn exec_info_json(&self) -> String {
        let mut spec = serde_json::json!({ "interactive": false });
        if let Some(cluster) = &self.cluster_info {
            spec["cluster"] = cluster.clone();
        }
        serde_json::json!({
            "apiVersion": self.api_version,
            "kind": "ExecCredential",
            "spec": spec,
        })
        .to_string()
    }
}

/// A credential returned by a plugin. `Debug` never shows secret values.
#[derive(Debug, Clone)]
pub struct Credential {
    pub token: Option<SecretString>,
    pub client_certificate_data: Option<String>,
    pub client_key_data: Option<SecretString>,
    pub expires_at: Option<DateTime<Utc>>,
}

impl Credential {
    /// Every secret literal held, for exact-match redaction.
    pub fn secret_literals(&self) -> Vec<String> {
        [self.token.as_ref(), self.client_key_data.as_ref()]
            .into_iter()
            .flatten()
            .map(|s| s.expose_secret().to_owned())
            .collect()
    }
}

/// Why an invocation failed, before it is mapped to a protocol [`PluginError`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunFailure {
    NotFound,
    NotExecutable(PathBuf),
    Spawn(String),
    NonZero { code: Option<i32>, stderr: String },
    Timeout(Duration),
    BadOutput(String),
    Expired(DateTime<Utc>),
    Interactive,
}

#[derive(Deserialize)]
struct ExecCredentialJson {
    status: Option<StatusJson>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct StatusJson {
    token: Option<String>,
    expiration_timestamp: Option<String>,
    client_certificate_data: Option<String>,
    client_key_data: Option<String>,
}

/// Parse an `ExecCredential` document. Errors never echo the document.
pub fn parse_credential(stdout: &[u8], now: DateTime<Utc>) -> Result<Credential, RunFailure> {
    let parsed: ExecCredentialJson = serde_json::from_slice(stdout).map_err(|_| {
        RunFailure::BadOutput(
            "plugin did not print a valid ExecCredential JSON document".to_owned(),
        )
    })?;
    let status = parsed
        .status
        .ok_or_else(|| RunFailure::BadOutput("ExecCredential has no status".to_owned()))?;
    let token = status
        .token
        .filter(|t| !t.is_empty())
        .map(SecretString::from);
    let cert = status.client_certificate_data.filter(|c| !c.is_empty());
    let key = status
        .client_key_data
        .filter(|k| !k.is_empty())
        .map(SecretString::from);
    if token.is_none() && (cert.is_none() || key.is_none()) {
        return Err(RunFailure::BadOutput(
            "ExecCredential status has neither a token nor a client certificate and key".to_owned(),
        ));
    }
    let expires_at = match status.expiration_timestamp.as_deref() {
        Some(ts) => Some(
            DateTime::parse_from_rfc3339(ts)
                .map_err(|_| {
                    RunFailure::BadOutput(
                        "ExecCredential expirationTimestamp is not RFC 3339".to_owned(),
                    )
                })?
                .with_timezone(&Utc),
        ),
        None => None,
    };
    if let Some(exp) = expires_at
        && exp <= now
    {
        return Err(RunFailure::Expired(exp));
    }
    Ok(Credential {
        token,
        client_certificate_data: cert,
        client_key_data: key,
        expires_at,
    })
}

async fn read_capped<R: AsyncRead + Unpin>(reader: Option<R>, cap: usize) -> (Vec<u8>, bool) {
    let Some(reader) = reader else {
        return (Vec::new(), false);
    };
    let mut buf = Vec::new();
    let mut limited = reader.take(cap as u64 + 1);
    let _ = limited.read_to_end(&mut buf).await;
    let over = buf.len() > cap;
    buf.truncate(cap);
    (buf, over)
}

/// Run the plugin once and return its parsed credential.
pub async fn run_plugin(
    spec: &ExecSpec,
    search: &[PathBuf],
    timeout: Duration,
) -> Result<Credential, RunFailure> {
    if spec.interactive_always {
        return Err(RunFailure::Interactive);
    }
    let path = match detect::resolve_command(&spec.command, search) {
        Resolution::Found(p) => p,
        Resolution::NotExecutable(p) => return Err(RunFailure::NotExecutable(p)),
        Resolution::NotFound => return Err(RunFailure::NotFound),
    };

    let mut cmd = tokio::process::Command::new(&path);
    cmd.args(&spec.args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    if let Ok(joined) = std::env::join_paths(search) {
        cmd.env("PATH", joined);
    }
    for (k, v) in &spec.env {
        cmd.env(k, v);
    }
    cmd.env("KUBERNETES_EXEC_INFO", spec.exec_info_json());
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW

    let mut child = cmd.spawn().map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => RunFailure::NotFound,
        std::io::ErrorKind::PermissionDenied => RunFailure::NotExecutable(path.clone()),
        _ => RunFailure::Spawn(e.to_string()),
    })?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    let work = async {
        let ((out, out_over), (err, _)) = tokio::join!(
            read_capped(stdout, STDOUT_CAP),
            read_capped(stderr, STDERR_CAP)
        );
        let status = child.wait().await;
        (out, out_over, err, status)
    };
    let Ok((out, out_over, err, status)) = tokio::time::timeout(timeout, work).await else {
        let _ = child.start_kill();
        return Err(RunFailure::Timeout(timeout));
    };
    let status = status.map_err(|e| RunFailure::Spawn(e.to_string()))?;
    if !status.success() {
        return Err(RunFailure::NonZero {
            code: status.code(),
            stderr: String::from_utf8_lossy(&err).into_owned(),
        });
    }
    if out_over {
        return Err(RunFailure::BadOutput(format!(
            "plugin output exceeded {} KiB",
            STDOUT_CAP / 1024
        )));
    }
    parse_credential(&out, Utc::now())
}

fn stderr_suggests_login(stderr: &str) -> bool {
    let s = stderr.to_ascii_lowercase();
    [
        "expired",
        "sso login",
        "aws sso",
        "auth login",
        "az login",
        "please log in",
        "please login",
        "login required",
        "not logged in",
        "reauth",
        "invalid_grant",
        "unable to locate credentials",
        "could not find default credentials",
    ]
    .iter()
    .any(|k| s.contains(k))
}

/// Map a runner failure to the protocol's typed error (T009). `secrets` are literal values that
/// must never appear in the message even if the plugin echoed them.
pub fn to_plugin_error(
    context: &str,
    provider: CloudProvider,
    command: &str,
    failure: &RunFailure,
    secrets: &[&str],
) -> PluginError {
    let relogin = || Some(detect::relogin_hint(provider).to_owned());
    let (kind, message, hint) = match failure {
        RunFailure::NotFound => (
            PluginErrorKind::PluginMissing,
            format!(
                "`{command}` was not found on PATH or in the usual install locations (a GUI-launched app may not see your login-shell PATH)"
            ),
            Some(detect::install_hint(provider, command)),
        ),
        RunFailure::NotExecutable(p) => (
            PluginErrorKind::PluginMissing,
            format!("`{}` exists but is not executable", p.display()),
            Some(detect::install_hint(provider, command)),
        ),
        RunFailure::Spawn(e) => (
            PluginErrorKind::PluginFailed,
            format!("could not start `{command}`: {e}"),
            None,
        ),
        RunFailure::NonZero { code, stderr } => {
            let kind = if stderr_suggests_login(stderr) {
                PluginErrorKind::CredentialExpired
            } else {
                PluginErrorKind::PluginFailed
            };
            let code = code.map_or_else(|| "a signal".to_owned(), |c| format!("exit code {c}"));
            let stderr = stderr.trim();
            let message = if stderr.is_empty() {
                format!("`{command}` failed with {code} and printed nothing on stderr")
            } else {
                format!("`{command}` failed with {code}: {stderr}")
            };
            let hint = (kind == PluginErrorKind::CredentialExpired)
                .then(|| detect::relogin_hint(provider).to_owned());
            (kind, message, hint)
        }
        RunFailure::Timeout(d) => (
            PluginErrorKind::PluginTimeout,
            format!(
                "`{command}` did not finish within {}s and was killed",
                d.as_secs()
            ),
            None,
        ),
        RunFailure::BadOutput(m) => (PluginErrorKind::PluginFailed, m.clone(), None),
        RunFailure::Expired(ts) => (
            PluginErrorKind::CredentialExpired,
            format!(
                "`{command}` returned a credential that expired at {}",
                ts.to_rfc3339()
            ),
            relogin(),
        ),
        RunFailure::Interactive => (
            PluginErrorKind::PluginInteractiveUnsupported,
            format!(
                "`{command}` requires an interactive terminal (interactiveMode: Always), which Fathom does not attach"
            ),
            Some(format!(
                "Log in with the provider CLI in a terminal first ({})",
                detect::relogin_hint(provider)
            )),
        ),
    };
    PluginError {
        kind,
        context: context.to_owned(),
        provider,
        command: command.to_owned(),
        message: redact::sanitize(&message, PLUGIN_ERROR_MAX_MESSAGE_BYTES, secrets),
        hint,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(cmd: &str) -> ExecSpec {
        ExecSpec {
            command: cmd.to_owned(),
            args: vec![],
            env: vec![],
            api_version: DEFAULT_API_VERSION.to_owned(),
            interactive_always: false,
            provide_cluster_info: false,
            cluster_info: None,
        }
    }

    #[tokio::test]
    async fn missing_command_maps_to_plugin_missing() {
        let f = run_plugin(&spec("no-such-plugin-xyz"), &[], DEFAULT_TIMEOUT)
            .await
            .unwrap_err();
        assert_eq!(f, RunFailure::NotFound);
        let e = to_plugin_error("ctx", CloudProvider::Gke, "gke-gcloud-auth-plugin", &f, &[]);
        assert_eq!(e.kind, PluginErrorKind::PluginMissing);
        assert!(e.hint.unwrap().contains("gcloud components install"));
        assert!(e.message.contains("gke-gcloud-auth-plugin"));
    }

    #[tokio::test]
    async fn interactive_always_is_refused_before_spawn() {
        let mut s = spec("whatever");
        s.interactive_always = true;
        assert_eq!(
            run_plugin(&s, &[], DEFAULT_TIMEOUT).await.unwrap_err(),
            RunFailure::Interactive
        );
        let e = to_plugin_error(
            "c",
            CloudProvider::Aks,
            "kubelogin",
            &RunFailure::Interactive,
            &[],
        );
        assert_eq!(e.kind, PluginErrorKind::PluginInteractiveUnsupported);
        assert!(e.hint.unwrap().contains("Log in"));
    }

    #[test]
    fn non_zero_exit_carries_redacted_truncated_stderr() {
        let f = RunFailure::NonZero {
            code: Some(2),
            stderr: format!("bad token k8s-aws-v1.SECRETSECRET {}", "x ".repeat(4000)),
        };
        let e = to_plugin_error("c", CloudProvider::Eks, "aws", &f, &[]);
        assert_eq!(e.kind, PluginErrorKind::PluginFailed);
        assert!(!e.message.contains("SECRETSECRET"));
        assert!(e.message.len() <= PLUGIN_ERROR_MAX_MESSAGE_BYTES);
        assert!(e.message.contains("exit code 2"));
    }

    #[test]
    fn login_stderr_maps_to_credential_expired_with_hint() {
        let f = RunFailure::NonZero {
            code: Some(255),
            stderr: "The SSO session associated with this profile has expired. aws sso login"
                .into(),
        };
        let e = to_plugin_error("c", CloudProvider::Eks, "aws", &f, &[]);
        assert_eq!(e.kind, PluginErrorKind::CredentialExpired);
        assert!(e.hint.unwrap().contains("aws sso login"));
    }

    #[test]
    fn timeout_and_expired_and_bad_output_kinds() {
        let t = to_plugin_error(
            "c",
            CloudProvider::Gke,
            "p",
            &RunFailure::Timeout(Duration::from_secs(20)),
            &[],
        );
        assert_eq!(t.kind, PluginErrorKind::PluginTimeout);
        assert!(t.message.contains("20s"));
        let x = to_plugin_error(
            "c",
            CloudProvider::Aks,
            "p",
            &RunFailure::Expired(Utc::now()),
            &[],
        );
        assert_eq!(x.kind, PluginErrorKind::CredentialExpired);
        let b = to_plugin_error(
            "c",
            CloudProvider::Aks,
            "p",
            &RunFailure::BadOutput("x".into()),
            &[],
        );
        assert_eq!(b.kind, PluginErrorKind::PluginFailed);
        let s = to_plugin_error(
            "c",
            CloudProvider::Aks,
            "p",
            &RunFailure::Spawn("e".into()),
            &[],
        );
        assert_eq!(s.kind, PluginErrorKind::PluginFailed);
        let n = to_plugin_error(
            "c",
            CloudProvider::Aks,
            "p",
            &RunFailure::NotExecutable("/x/p".into()),
            &[],
        );
        assert_eq!(n.kind, PluginErrorKind::PluginMissing);
    }

    #[test]
    fn parses_credentials_and_rejects_bad_ones() {
        let now = Utc::now();
        let future = (now + chrono::Duration::hours(1)).to_rfc3339();
        let ok = format!(
            r#"{{"apiVersion":"client.authentication.k8s.io/v1","kind":"ExecCredential","status":{{"token":"t0k3n","expirationTimestamp":"{future}"}}}}"#
        );
        let c = parse_credential(ok.as_bytes(), now).unwrap();
        assert_eq!(c.token.as_ref().unwrap().expose_secret(), "t0k3n");
        assert!(c.expires_at.is_some());
        assert!(
            !format!("{c:?}").contains("t0k3n"),
            "Debug must not leak the token"
        );

        let past = (now - chrono::Duration::hours(1)).to_rfc3339();
        let expired = format!(r#"{{"status":{{"token":"a","expirationTimestamp":"{past}"}}}}"#);
        assert!(matches!(
            parse_credential(expired.as_bytes(), now),
            Err(RunFailure::Expired(_))
        ));
        assert!(matches!(
            parse_credential(b"nope", now),
            Err(RunFailure::BadOutput(_))
        ));
        assert!(matches!(
            parse_credential(br#"{"status":{}}"#, now),
            Err(RunFailure::BadOutput(_))
        ));
    }

    #[test]
    fn known_only_policy_keeps_only_allowlisted_env() {
        let mut s = spec("aws");
        s.env = vec![
            ("LD_PRELOAD".into(), "/tmp/evil.so".into()),
            ("AWS_PROFILE".into(), "prod".into()),
            ("AWS_REGION".into(), "us-east-1".into()),
        ];
        let restricted = s.clone().restricted_to(ExecPolicy::AllowKnownOnly);
        assert_eq!(
            restricted.env,
            vec![
                ("AWS_PROFILE".to_owned(), "prod".to_owned()),
                ("AWS_REGION".to_owned(), "us-east-1".to_owned()),
            ]
        );
        assert_eq!(s.clone().restricted_to(ExecPolicy::AllowAll).env, s.env);
    }

    #[test]
    fn identity_distinguishes_args_env_and_cluster_info() {
        let a = spec("aws");
        let mut b = spec("aws");
        b.args = vec!["x".into()];
        let mut c = spec("aws");
        c.env = vec![("AWS_PROFILE".into(), "p".into())];
        assert_ne!(a.identity(), b.identity());
        assert_ne!(a.identity(), c.identity());
        assert_eq!(a.identity(), spec("aws").identity());
        let mut d = spec("aws");
        d.provide_cluster_info = true;
        d.cluster_info = Some(serde_json::json!({"server": "https://a"}));
        let mut e = d.clone();
        e.cluster_info = Some(serde_json::json!({"server": "https://b"}));
        assert_ne!(d.identity(), e.identity());
    }
}
