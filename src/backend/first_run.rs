//! First-run and connection-failure classification.
//!
//! Before this module, a missing, empty or malformed kubeconfig, an unreachable server or a
//! rejected token all reached the UI as one raw error string (often the multi-line "Failed to
//! infer configuration" wall), and the UI could not tell them apart. The backend now classifies
//! each case into a small enum the UI can explain with a next step:
//!
//! - [`KubeconfigIssue`] (built by `kube::resolve_kubeconfig`): missing, empty, malformed,
//!   unreadable, or no current context.
//! - [`ConnectionProblem`] ([`classify_connection_error`]): unreachable, DNS, timeout, TLS, 401,
//!   403, other.
//!
//! It also holds the process-wide "active cluster is down" flag that keeps a cluster outage from
//! turning every 20-second background poll into another error notification.

use std::error::Error as StdError;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::protocol::{
    ConnectionProblem, ConnectionProblemKind, KubeconfigIssue, KubeconfigIssueKind,
};

/// Longest detail kept from a parser or connection error.
const MAX_DETAIL: usize = 300;

/// First line of `text`, capped at [`MAX_DETAIL`] characters. YAML parser errors can run to many
/// lines; the first names the location, which is what a person needs.
pub(super) fn short_detail(text: &str) -> String {
    let first = text
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim();
    if first.chars().count() <= MAX_DETAIL {
        return first.to_owned();
    }
    let mut out: String = first.chars().take(MAX_DETAIL).collect();
    out.push('…');
    out
}

/// One-line English description of a kubeconfig issue (logs and command errors).
pub(super) fn describe_issue(issue: &KubeconfigIssue) -> String {
    let paths = issue.paths.join(", ");
    let base = match issue.kind {
        KubeconfigIssueKind::Missing if paths.is_empty() => {
            "No kubeconfig found (KUBECONFIG is unset and there is no home directory)".to_owned()
        }
        KubeconfigIssueKind::Missing => format!("No kubeconfig found at {paths}"),
        KubeconfigIssueKind::Empty => format!("The kubeconfig {paths} defines no contexts"),
        KubeconfigIssueKind::Malformed => format!("The kubeconfig {paths} is not valid"),
        KubeconfigIssueKind::Unreadable => format!("The kubeconfig {paths} cannot be read"),
        KubeconfigIssueKind::NoCurrentContext => "No kubeconfig context is selected".to_owned(),
    };
    if issue.detail.is_empty() {
        base
    } else {
        format!("{base}: {}", issue.detail)
    }
}

/// Classify a connection failure from its status code (if any) and its full error text.
pub(super) fn classify(code: Option<u16>, text: &str) -> ConnectionProblemKind {
    match code {
        Some(401) => return ConnectionProblemKind::Unauthorized,
        Some(403) => return ConnectionProblemKind::Forbidden,
        _ => {}
    }
    let t = text.to_ascii_lowercase();
    let any = |needles: &[&str]| needles.iter().any(|n| t.contains(n));
    if any(&["unauthorized"]) {
        ConnectionProblemKind::Unauthorized
    } else if any(&["forbidden"]) {
        ConnectionProblemKind::Forbidden
    } else if any(&[
        "dns error",
        "failed to lookup address",
        "name or service not known",
        "no such host",
        "temporary failure in name resolution",
        "nodename nor servname",
    ]) {
        ConnectionProblemKind::Dns
    } else if any(&[
        "certificate",
        "invalidcertificate",
        "unknownissuer",
        "tls handshake",
        "tls error",
        "handshake failure",
    ]) {
        ConnectionProblemKind::Tls
    } else if any(&["timed out", "timeout", "deadline has elapsed"]) {
        ConnectionProblemKind::Timeout
    } else if any(&[
        "connection refused",
        "no route to host",
        "network is unreachable",
        "host is down",
        "connection reset",
        "broken pipe",
        "error trying to connect",
        "client error (connect)",
        "tcp connect error",
    ]) {
        ConnectionProblemKind::Unreachable
    } else {
        ConnectionProblemKind::Other
    }
}

/// Walk an error chain for an API status code and the joined messages, then [`classify`].
pub(super) fn classify_std(err: &(dyn StdError + 'static)) -> (ConnectionProblemKind, String) {
    let mut code = None;
    let mut parts: Vec<String> = Vec::new();
    let mut cur: Option<&(dyn StdError + 'static)> = Some(err);
    while let Some(e) = cur {
        if code.is_none() {
            if let Some(kube::Error::Api(resp)) = e.downcast_ref::<kube::Error>() {
                code = Some(resp.code);
            } else if let Some(resp) = e.downcast_ref::<kube::core::Status>() {
                code = Some(resp.code);
            } else if let Some(resp) = e.downcast_ref::<Box<kube::core::Status>>() {
                // kube 4's `Error::Api` sources its boxed `Status`, which is this type, not `Status`.
                code = Some(resp.code);
            }
        }
        let msg = e.to_string();
        if !parts.iter().any(|p| p.contains(&msg)) {
            parts.push(msg);
        }
        cur = e.source();
    }
    let text = parts.join(": ");
    (classify(code, &text), text)
}

/// Classify an `anyhow` error from a connect attempt.
pub(super) fn classify_connection_error(
    err: &anyhow::Error,
    server: Option<String>,
) -> ConnectionProblem {
    let (kind, _) = classify_std(err.as_ref());
    ConnectionProblem {
        kind,
        server,
        detail: format!("{err:#}"),
    }
}

/// The server URL of `context` in the current kubeconfig, for the UI's "cannot reach <server>".
pub(super) fn server_for_context(context: &str) -> Option<String> {
    let kc = super::kube::load_kubeconfig().ok()?;
    let cluster_name = kc
        .contexts
        .iter()
        .find(|c| c.name == context)?
        .context
        .as_ref()?
        .cluster
        .clone();
    kc.clusters
        .iter()
        .find(|c| c.name == cluster_name)?
        .cluster
        .as_ref()?
        .server
        .clone()
}

/// True when the connection problem means the cluster itself is gone or refusing us, so
/// background polls should stop reporting their own copies of the same failure.
pub(super) fn is_connection_level(kind: ConnectionProblemKind) -> bool {
    !matches!(
        kind,
        ConnectionProblemKind::Other | ConnectionProblemKind::Forbidden
    )
}

/// Set while the active cluster is known to be unreachable (or rejecting our credentials).
static ACTIVE_DOWN: AtomicBool = AtomicBool::new(false);

/// Record whether the active cluster is down. Returns the previous value, so a caller can act on
/// the transition only.
pub(super) fn set_active_down(down: bool) -> bool {
    ACTIVE_DOWN.swap(down, Ordering::SeqCst)
}

/// Whether the active cluster is currently known to be down.
pub(super) fn active_down() -> bool {
    ACTIVE_DOWN.load(Ordering::SeqCst)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::kube::{
        current_context_issue, kubeconfig_search_paths, resolve_kubeconfig,
    };
    use std::ffi::OsString;
    use std::path::PathBuf;

    const GOOD: &str = "apiVersion: v1
kind: Config
clusters:
- name: c
  cluster:
    server: https://127.0.0.1:6443
users:
- name: u
  user:
    token: t
contexts:
- name: ctx-a
  context: {cluster: c, user: u}
current-context: ctx-a
";

    const OTHER: &str = "apiVersion: v1
kind: Config
clusters:
- name: c2
  cluster:
    server: https://127.0.0.1:7443
users:
- name: u2
  user:
    token: t
contexts:
- name: ctx-b
  context: {cluster: c2, user: u2}
current-context: ctx-b
";

    fn write(dir: &std::path::Path, name: &str, body: &str) -> PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, body).unwrap();
        p
    }

    fn env_of(paths: &[&PathBuf]) -> Option<OsString> {
        Some(std::env::join_paths(paths.iter().map(|p| p.as_os_str())).unwrap())
    }

    #[test]
    fn missing_kubeconfig_env_is_missing_and_never_falls_back_to_home() {
        let dir = tempfile::tempdir().unwrap();
        // A perfectly good home kubeconfig must be ignored while KUBECONFIG is set.
        std::fs::create_dir_all(dir.path().join(".kube")).unwrap();
        write(&dir.path().join(".kube"), "config", GOOD);
        let missing = dir.path().join("nope.yaml");
        let err = resolve_kubeconfig(env_of(&[&missing]), Some(dir.path().to_owned())).unwrap_err();
        assert_eq!(err.kind, KubeconfigIssueKind::Missing);
        assert_eq!(err.paths, vec![missing.display().to_string()]);
    }

    #[test]
    fn unset_env_and_empty_home_is_missing_with_the_home_path() {
        let dir = tempfile::tempdir().unwrap();
        let err = resolve_kubeconfig(None, Some(dir.path().to_owned())).unwrap_err();
        assert_eq!(err.kind, KubeconfigIssueKind::Missing);
        let expected = dir.path().join(".kube").join("config");
        assert_eq!(err.paths, vec![expected.display().to_string()]);
        assert!(describe_issue(&err).contains(".kube"));
    }

    #[test]
    fn empty_env_value_means_the_home_kubeconfig() {
        let paths = kubeconfig_search_paths(Some(OsString::from("")), Some(PathBuf::from("/h")));
        assert_eq!(paths, vec![PathBuf::from("/h/.kube/config")]);
    }

    #[test]
    fn no_env_and_no_home_is_missing_without_paths() {
        let err = resolve_kubeconfig(None, None).unwrap_err();
        assert_eq!(err.kind, KubeconfigIssueKind::Missing);
        assert!(err.paths.is_empty());
        assert!(describe_issue(&err).contains("no home directory"));
    }

    #[test]
    fn empty_file_is_empty_not_missing() {
        let dir = tempfile::tempdir().unwrap();
        let p = write(dir.path(), "empty.yaml", "");
        let err = resolve_kubeconfig(env_of(&[&p]), None).unwrap_err();
        assert_eq!(err.kind, KubeconfigIssueKind::Empty);
        assert_eq!(err.paths, vec![p.display().to_string()]);
    }

    #[test]
    fn config_without_contexts_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        let p = write(dir.path(), "bare.yaml", "apiVersion: v1\nkind: Config\n");
        let err = resolve_kubeconfig(env_of(&[&p]), None).unwrap_err();
        assert_eq!(err.kind, KubeconfigIssueKind::Empty);
    }

    #[test]
    fn malformed_file_is_malformed_with_a_short_detail() {
        let dir = tempfile::tempdir().unwrap();
        let p = write(
            dir.path(),
            "bad.yaml",
            "apiVersion: v1\nclusters: [ {name: x, cluster: {server:\n  - broken\n contexts: :::\n",
        );
        let err = resolve_kubeconfig(env_of(&[&p]), None).unwrap_err();
        assert_eq!(err.kind, KubeconfigIssueKind::Malformed);
        assert_eq!(err.paths, vec![p.display().to_string()]);
        assert!(!err.detail.is_empty());
        assert!(!err.detail.contains('\n'));
        assert!(err.detail.chars().count() <= MAX_DETAIL + 1);
    }

    #[test]
    fn malformed_file_is_not_skipped_in_favour_of_a_later_good_one() {
        let dir = tempfile::tempdir().unwrap();
        let bad = write(dir.path(), "bad.yaml", "clusters: [ {");
        let good = write(dir.path(), "good.yaml", GOOD);
        let err = resolve_kubeconfig(env_of(&[&bad, &good]), None).unwrap_err();
        assert_eq!(err.kind, KubeconfigIssueKind::Malformed);
    }

    #[test]
    fn directory_is_unreadable() {
        let dir = tempfile::tempdir().unwrap();
        let err = resolve_kubeconfig(env_of(&[&dir.path().to_owned()]), None).unwrap_err();
        assert_eq!(err.kind, KubeconfigIssueKind::Unreadable);
    }

    #[test]
    fn env_list_merges_like_kubectl_and_skips_missing_entries() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("missing.yaml");
        let a = write(dir.path(), "a.yaml", GOOD);
        let b = write(dir.path(), "b.yaml", OTHER);
        let (kc, used) = resolve_kubeconfig(env_of(&[&missing, &a, &b]), None).unwrap();
        let names: Vec<_> = kc.contexts.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, vec!["ctx-a", "ctx-b"]);
        // The first file to set current-context wins.
        assert_eq!(kc.current_context.as_deref(), Some("ctx-a"));
        assert_eq!(used, vec![a, b]);
    }

    #[test]
    fn current_context_issues() {
        let contexts = vec!["a".to_owned(), "b".to_owned()];
        let none = current_context_issue(&contexts, None, &[]).unwrap();
        assert_eq!(none.kind, KubeconfigIssueKind::NoCurrentContext);
        assert!(none.detail.is_empty());
        let unknown = current_context_issue(&contexts, Some("gone"), &[]).unwrap();
        assert_eq!(unknown.kind, KubeconfigIssueKind::NoCurrentContext);
        assert!(unknown.detail.contains("gone"));
        assert!(current_context_issue(&contexts, Some("a"), &[]).is_none());
    }

    #[test]
    fn short_detail_keeps_the_first_line_and_caps_length() {
        assert_eq!(short_detail("\n  first line \nsecond"), "first line");
        let long = "x".repeat(1000);
        let d = short_detail(&long);
        assert_eq!(d.chars().count(), MAX_DETAIL + 1);
        assert!(d.ends_with('…'));
    }

    #[test]
    fn classifies_status_codes() {
        assert_eq!(classify(Some(401), ""), ConnectionProblemKind::Unauthorized);
        assert_eq!(classify(Some(403), ""), ConnectionProblemKind::Forbidden);
        assert_eq!(
            classify(
                None,
                "ApiError: Unauthorized: Unauthorized (ErrorResponse { code: 401 })"
            ),
            ConnectionProblemKind::Unauthorized
        );
    }

    #[test]
    fn classifies_transport_failures() {
        let cases = [
            (
                "ServiceError: client error (Connect): tcp connect error: Connection refused (os error 111)",
                ConnectionProblemKind::Unreachable,
            ),
            (
                "failed to perform initial object list: ServiceError: client error (Connect)",
                ConnectionProblemKind::Unreachable,
            ),
            (
                "client error (Connect): dns error: failed to lookup address information: Name or service not known",
                ConnectionProblemKind::Dns,
            ),
            (
                "client error (Connect): invalid peer certificate: UnknownIssuer",
                ConnectionProblemKind::Tls,
            ),
            (
                "Timed out after 5s probing context",
                ConnectionProblemKind::Timeout,
            ),
            ("something else entirely", ConnectionProblemKind::Other),
        ];
        for (text, want) in cases {
            assert_eq!(classify(None, text), want, "{text}");
        }
    }

    #[test]
    fn classifies_a_kube_api_error_through_an_anyhow_chain() {
        let api = kube::Error::Api(
            kube::core::Status::failure("Unauthorized", "Unauthorized")
                .with_code(401)
                .boxed(),
        );
        let err = anyhow::Error::new(api).context("GET /version");
        let p = classify_connection_error(&err, Some("https://x:6443".into()));
        assert_eq!(p.kind, ConnectionProblemKind::Unauthorized);
        assert_eq!(p.server.as_deref(), Some("https://x:6443"));
        assert!(p.detail.contains("GET /version"));

        let forbidden = kube::Error::Api(
            kube::core::Status::failure("nope", "Forbidden")
                .with_code(403)
                .boxed(),
        );
        let (kind, _) = classify_std(&forbidden);
        assert_eq!(kind, ConnectionProblemKind::Forbidden);

        // The status alone, as the next link of a chain sees it, boxed (kube 4's `source`) or not.
        let source = forbidden.source().expect("Error::Api sources its Status");
        assert_eq!(classify_std(source).0, ConnectionProblemKind::Forbidden);
        let bare = kube::core::Status::failure("nope", "Forbidden").with_code(403);
        assert_eq!(classify_std(&bare).0, ConnectionProblemKind::Forbidden);
    }

    #[test]
    fn connection_level_kinds() {
        assert!(is_connection_level(ConnectionProblemKind::Unreachable));
        assert!(is_connection_level(ConnectionProblemKind::Unauthorized));
        assert!(!is_connection_level(ConnectionProblemKind::Forbidden));
        assert!(!is_connection_level(ConnectionProblemKind::Other));
    }
}
