//! 045 US3/US6: read the `kube-apiserver` static pod's authentication configuration, compare it
//! with the issuer being configured, and — only for a cluster Fathom provisioned — write it.
//!
//! Everything here is split so the decisions are testable without a cluster: parsing, the
//! unknown-vs-known read, the comparison and the manifest rewrite are pure functions, and the
//! two things that need a node (running a command, writing a file) sit behind [`NodeExec`].

use std::time::Instant;

use crossbeam_channel::Sender;
use k8s_openapi::api::core::v1::Pod;
use kube::{Api, api::ListParams};

use crate::backend::capabilities::CAPABILITY_TTL;
use crate::backend::events;
use crate::backend::persistence::KindProvenanceFile;
use crate::backend::state::BackendState;
use crate::protocol::{
    ApiServerAuthComparison, ApiServerAuthRead, ApiServerAuthStyle, AuthDifference, BackendEvent,
    BackendEventEnvelope, Correlation, KindRuntimeKind, OidcConfig, OidcIssuerConfig,
    ResourceReadEvent, Route, section_keys,
};

/// Label selector that finds the API server static pod on any kubeadm-style control plane
/// (kind included).
pub const APISERVER_POD_SELECTOR: &str = "component=kube-apiserver";

/// Path of the static pod manifest on the control-plane node.
pub const MANIFEST_PATH: &str = "/etc/kubernetes/manifests/kube-apiserver.yaml";
/// Where the pre-change manifest is kept. FR-021 wants a backup taken *before* any mutation, so
/// a failed write can be undone without the operator reconstructing the file by hand.
pub const BACKUP_PATH: &str = "/etc/kubernetes/kube-apiserver.yaml.fathom-backup";
/// Where a structured `AuthenticationConfiguration` is written, beside the other API server
/// files that are already mounted into the static pod.
pub const AUTH_CONFIG_PATH: &str = "/etc/kubernetes/pki/fathom-authentication-config.yaml";

// ---------------------------------------------------------------------------
// T033: parsing the API server's command line
// ---------------------------------------------------------------------------

/// What one `kube-apiserver` command line says about authentication.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedAuth {
    pub style: ApiServerAuthStyle,
    pub issuer: Option<String>,
    pub client_id: Option<String>,
    pub username_claim: Option<String>,
    pub groups_claim: Option<String>,
    pub username_prefix: Option<String>,
    pub groups_prefix: Option<String>,
    /// Path given to `--authentication-config`, when that is the style in use.
    pub config_path: Option<String>,
}

impl Default for ParsedAuth {
    /// Nothing read yet is `Unknown`, not "no OIDC": the difference between the two is the whole
    /// point of FR-010, and a default that said `None` would be an assumed pass.
    fn default() -> Self {
        Self {
            style: ApiServerAuthStyle::Unknown,
            issuer: None,
            client_id: None,
            username_claim: None,
            groups_claim: None,
            username_prefix: None,
            groups_prefix: None,
            config_path: None,
        }
    }
}

/// The value of `--flag=value` or `--flag value`, whichever form the manifest uses.
#[must_use]
pub fn flag_value(argv: &[String], flag: &str) -> Option<String> {
    let prefix = format!("--{flag}=");
    let bare = format!("--{flag}");
    let mut it = argv.iter();
    while let Some(arg) = it.next() {
        if let Some(v) = arg.strip_prefix(&prefix) {
            return Some(v.to_owned());
        }
        if arg == &bare {
            // The separated form: the value is the next argument, and an empty tail means the
            // flag was given without one, which is not a value we can report.
            return it.next().cloned();
        }
    }
    None
}

/// Classify a `kube-apiserver` command line.
///
/// The two styles are mutually exclusive in Kubernetes itself (the API server refuses to start
/// with both), so a manifest carrying both is not a configuration we pretend to understand: it
/// is `Unknown`, which the UI renders as a stated unknown rather than as agreement.
#[must_use]
pub fn parse_command_line(argv: &[String]) -> ParsedAuth {
    let structured = flag_value(argv, "authentication-config");
    let issuer = flag_value(argv, "oidc-issuer-url");

    match (structured, issuer) {
        (Some(path), None) => ParsedAuth {
            style: ApiServerAuthStyle::Structured,
            config_path: Some(path),
            ..ParsedAuth::default()
        },
        (None, Some(issuer)) => ParsedAuth {
            style: ApiServerAuthStyle::Legacy,
            issuer: Some(issuer),
            client_id: flag_value(argv, "oidc-client-id"),
            username_claim: flag_value(argv, "oidc-username-claim"),
            groups_claim: flag_value(argv, "oidc-groups-claim"),
            username_prefix: flag_value(argv, "oidc-username-prefix"),
            groups_prefix: flag_value(argv, "oidc-groups-prefix"),
            config_path: None,
        },
        (Some(path), Some(_)) => ParsedAuth {
            style: ApiServerAuthStyle::Unknown,
            config_path: Some(path),
            ..ParsedAuth::default()
        },
        (None, None) => ParsedAuth {
            style: ApiServerAuthStyle::None,
            ..ParsedAuth::default()
        },
    }
}

// ---------------------------------------------------------------------------
// T034/T035: the read
// ---------------------------------------------------------------------------

/// Build an [`ApiServerAuthRead`] from whatever the pod list came back as.
///
/// A denied or impossible read is a *successful* command: `checked: false`, `style: Unknown` and
/// the API server's own words in `reason` (FR-010). It is never an error and never an assumed
/// pass, following `detect_kubeadm`'s three-state precedent in `rbac_access.rs`.
#[must_use]
pub fn read_from_pods(pods: Result<Vec<Pod>, String>, context: &str) -> ApiServerAuthRead {
    let unknown = |reason: String| ApiServerAuthRead {
        style: ApiServerAuthStyle::Unknown,
        issuer: None,
        client_id: None,
        username_claim: None,
        groups_claim: None,
        username_prefix: None,
        groups_prefix: None,
        context: context.to_owned(),
        checked: false,
        reason: Some(reason),
    };

    let pods = match pods {
        Ok(pods) => pods,
        // Verbatim: an RBAC denial reads "pods is forbidden: User ... cannot list resource ..."
        // and the operator needs those words, not a paraphrase, to fix it.
        Err(e) => return unknown(e),
    };

    let Some(argv) = pods.iter().find_map(apiserver_argv) else {
        return unknown(
            "No kube-apiserver pod is visible in kube-system, so its authentication flags \
             could not be read. A managed control plane does not expose one."
                .to_owned(),
        );
    };

    let parsed = parse_command_line(&argv);
    match parsed.style {
        // The structured file lives on the node's filesystem and is not reachable through the
        // Kubernetes API, so its contents are genuinely unknown from here. Saying so is the
        // honest answer; reporting "no differences" would be an assumed pass.
        ApiServerAuthStyle::Structured => ApiServerAuthRead {
            style: ApiServerAuthStyle::Structured,
            context: context.to_owned(),
            checked: false,
            reason: Some(format!(
                "This API server authenticates with --authentication-config={}, a file on the \
                 control-plane node that the Kubernetes API does not expose. Its issuer cannot \
                 be compared from here; the legacy --oidc-* flags are not in use.",
                parsed.config_path.as_deref().unwrap_or(AUTH_CONFIG_PATH)
            )),
            issuer: None,
            client_id: None,
            username_claim: None,
            groups_claim: None,
            username_prefix: None,
            groups_prefix: None,
        },
        ApiServerAuthStyle::Unknown => ApiServerAuthRead {
            style: ApiServerAuthStyle::Unknown,
            context: context.to_owned(),
            checked: false,
            reason: Some(
                "This API server's manifest sets both --authentication-config and \
                 --oidc-issuer-url, which Kubernetes treats as mutually exclusive. Fathom will \
                 not guess which one is in effect."
                    .to_owned(),
            ),
            issuer: None,
            client_id: None,
            username_claim: None,
            groups_claim: None,
            username_prefix: None,
            groups_prefix: None,
        },
        style => ApiServerAuthRead {
            style,
            issuer: parsed.issuer,
            client_id: parsed.client_id,
            username_claim: parsed.username_claim,
            groups_claim: parsed.groups_claim,
            username_prefix: parsed.username_prefix,
            groups_prefix: parsed.groups_prefix,
            context: context.to_owned(),
            checked: true,
            reason: None,
        },
    }
}

/// The API server's effective argument vector: `command` carries the flags in a kubeadm static
/// pod, with `args` appended for the (unusual) manifests that split them.
fn apiserver_argv(pod: &Pod) -> Option<Vec<String>> {
    let containers = &pod.spec.as_ref()?.containers;
    let c = containers
        .iter()
        .find(|c| c.name == "kube-apiserver")
        .or_else(|| containers.first())?;
    let mut argv = c.command.clone().unwrap_or_default();
    argv.extend(c.args.clone().unwrap_or_default());
    (!argv.is_empty()).then_some(argv)
}

// ---------------------------------------------------------------------------
// T036: the comparison
// ---------------------------------------------------------------------------

/// Field-by-field differences between what the API server does and what `draft` asks for.
///
/// `None` on either side means *absent*, which the protocol keeps distinct from *different*:
/// "the cluster does not set `--oidc-groups-prefix` at all" and "it sets a different one" lead
/// the operator to different fixes, so they must stay distinguishable (FR-011).
///
/// An unchecked read yields no differences at all: nothing is known to differ, and an empty list
/// is only ever read as agreement alongside `checked: true`.
#[must_use]
pub fn compare(read: &ApiServerAuthRead, draft: &OidcIssuerConfig) -> Vec<AuthDifference> {
    if !read.checked {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut diff = |field: &str, cluster: Option<&String>, draft: Option<&String>| {
        // Trailing-slash-insensitive nowhere: the API server compares issuers exactly, so a
        // difference the operator cannot see is still a difference they must fix.
        if cluster.map(String::as_str) != draft.map(String::as_str) {
            out.push(AuthDifference {
                field: field.to_owned(),
                cluster: cluster.cloned(),
                draft: draft.cloned(),
            });
        }
    };
    diff("issuer", read.issuer.as_ref(), Some(&draft.issuer_url));
    diff("clientId", read.client_id.as_ref(), Some(&draft.client_id));
    diff(
        "usernameClaim",
        read.username_claim.as_ref(),
        Some(&draft.username_claim),
    );
    diff(
        "groupsClaim",
        read.groups_claim.as_ref(),
        Some(&draft.groups_claim),
    );
    diff(
        "usernamePrefix",
        read.username_prefix.as_ref(),
        draft.username_prefix.as_ref(),
    );
    diff(
        "groupsPrefix",
        read.groups_prefix.as_ref(),
        draft.groups_prefix.as_ref(),
    );
    out
}

// ---------------------------------------------------------------------------
// T059/T060: the write
// ---------------------------------------------------------------------------

/// What the operator confirmed should be written to the API server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesiredAuth {
    pub style: ApiServerAuthStyle,
    pub issuer_url: String,
    pub client_id: String,
    pub username_claim: String,
    pub groups_claim: String,
    pub username_prefix: Option<String>,
    pub groups_prefix: Option<String>,
}

/// Running a command on, and writing a file to, a control-plane node.
///
/// A seam, so the ordering FR-021 requires (backup, then mutate, then wait) can be asserted in a
/// unit test; the real implementation shells out to the container runtime.
pub trait NodeExec {
    fn run(&self, args: &[&str]) -> Result<String, String>;
    fn write_file(&self, path: &str, contents: &str) -> Result<(), String>;
}

/// A structured `AuthenticationConfiguration` for `--authentication-config`.
#[must_use]
pub fn authentication_config_yaml(d: &DesiredAuth) -> String {
    let mut y = String::from(
        "apiVersion: apiserver.config.k8s.io/v1beta1\nkind: AuthenticationConfiguration\njwt:\n",
    );
    y.push_str(&format!(
        "- issuer:\n    url: {}\n    audiences:\n    - {}\n",
        d.issuer_url, d.client_id
    ));
    y.push_str("  claimMappings:\n");
    let prefix = d.username_prefix.clone().unwrap_or_default();
    y.push_str(&format!(
        "    username:\n      claim: {}\n      prefix: {:?}\n",
        d.username_claim, prefix
    ));
    y.push_str(&format!(
        "    groups:\n      claim: {}\n      prefix: {:?}\n",
        d.groups_claim,
        d.groups_prefix.clone().unwrap_or_default()
    ));
    y
}

/// The legacy flag lines for a static pod manifest, at `indent`.
fn legacy_flag_lines(d: &DesiredAuth, indent: &str) -> Vec<String> {
    let mut lines = vec![
        format!("{indent}- --oidc-issuer-url={}", d.issuer_url),
        format!("{indent}- --oidc-client-id={}", d.client_id),
        format!("{indent}- --oidc-username-claim={}", d.username_claim),
        format!("{indent}- --oidc-groups-claim={}", d.groups_claim),
    ];
    if let Some(p) = &d.username_prefix {
        lines.push(format!("{indent}- --oidc-username-prefix={p}"));
    }
    if let Some(p) = &d.groups_prefix {
        lines.push(format!("{indent}- --oidc-groups-prefix={p}"));
    }
    lines
}

/// Rewrite a `kube-apiserver` static pod manifest to authenticate against `d`.
///
/// Both styles' flags are stripped before the chosen style's are added, so the result can never
/// carry both — a manifest that did would stop the API server from starting at all.
pub fn rewrite_manifest(manifest: &str, d: &DesiredAuth) -> Result<String, String> {
    let is_auth_flag =
        |t: &str| t.starts_with("- --oidc-") || t.starts_with("- --authentication-config=");
    // The indentation of the existing flag list, so the inserted lines sit in the same sequence.
    let anchor = manifest
        .lines()
        .find(|l| l.trim_start().starts_with("- --") && !is_auth_flag(l.trim_start()))
        .ok_or_else(|| {
            "Could not find the kube-apiserver command line in the static pod manifest; \
             refusing to rewrite a manifest Fathom does not recognise."
                .to_owned()
        })?;
    let indent = &anchor[..anchor.len() - anchor.trim_start().len()];

    let new_lines: Vec<String> = match d.style {
        ApiServerAuthStyle::Structured => {
            vec![format!(
                "{indent}- --authentication-config={AUTH_CONFIG_PATH}"
            )]
        }
        ApiServerAuthStyle::Legacy | ApiServerAuthStyle::None => legacy_flag_lines(d, indent),
        ApiServerAuthStyle::Unknown => {
            return Err(
                "Fathom will not write an API server configuration whose style it could not \
                 determine."
                    .to_owned(),
            );
        }
    };

    let mut out: Vec<String> = Vec::new();
    let mut inserted = false;
    for line in manifest.lines() {
        if is_auth_flag(line.trim_start()) {
            continue;
        }
        if !inserted && std::ptr::eq(line.as_ptr(), anchor.as_ptr()) {
            out.extend(new_lines.iter().cloned());
            inserted = true;
        }
        out.push(line.to_owned());
    }
    let mut joined = out.join("\n");
    if manifest.ends_with('\n') {
        joined.push('\n');
    }
    Ok(joined)
}

/// Apply `d` to the control-plane node behind `exec`, then wait for the API server to come back.
///
/// Order is the contract (FR-021): back the manifest up first, mutate second, wait third. If the
/// control plane does not come back the manifest is restored from the backup and the failure is
/// reported in the words the wait produced — a half-written control plane is worse than no
/// change at all.
pub fn apply_write(
    exec: &dyn NodeExec,
    d: &DesiredAuth,
    wait: &dyn Fn() -> Result<(), String>,
) -> Result<(), String> {
    // `-n` so a second run does not overwrite the operator's original with our own output.
    exec.run(&["cp", "-n", MANIFEST_PATH, BACKUP_PATH])?;
    let manifest = exec.run(&["cat", MANIFEST_PATH])?;
    let rewritten = rewrite_manifest(&manifest, d)?;

    if d.style == ApiServerAuthStyle::Structured {
        // The config file must exist before the flag pointing at it does, or the API server
        // crash-loops on a missing file.
        exec.write_file(AUTH_CONFIG_PATH, &authentication_config_yaml(d))?;
    }
    exec.write_file(MANIFEST_PATH, &rewritten)?;

    match wait() {
        Ok(()) => Ok(()),
        Err(e) => {
            let restored = exec.run(&["cp", BACKUP_PATH, MANIFEST_PATH]);
            Err(match restored {
                Ok(_) => format!(
                    "{e}\nThe previous kube-apiserver manifest has been restored from \
                     {BACKUP_PATH}."
                ),
                Err(r) => format!(
                    "{e}\nRestoring the previous manifest from {BACKUP_PATH} also failed: {r}"
                ),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::persistence::KindProvenanceRecord;
    use k8s_openapi::api::core::v1::{Container, PodSpec};
    use std::cell::RefCell;
    use std::time::Duration;

    fn pod(argv: &[&str]) -> Pod {
        Pod {
            spec: Some(PodSpec {
                containers: vec![Container {
                    name: "kube-apiserver".into(),
                    command: Some(argv.iter().map(|s| (*s).to_owned()).collect()),
                    ..Container::default()
                }],
                ..PodSpec::default()
            }),
            ..Pod::default()
        }
    }

    fn argv(args: &[&str]) -> Vec<String> {
        args.iter().map(|s| (*s).to_owned()).collect()
    }

    fn draft() -> OidcIssuerConfig {
        OidcIssuerConfig {
            id: "corp".into(),
            issuer_url: "https://idp.example.com/realms/corp".into(),
            client_id: "fathom".into(),
            scopes: vec!["openid".into()],
            ca_bundle_path: None,
            username_claim: "email".into(),
            groups_claim: "groups".into(),
            username_prefix: Some("oidc:".into()),
            groups_prefix: Some("oidc:".into()),
            admin_api_base: None,
        }
    }

    /// T033 (FR-009): both flag styles parse, in both `=` and separated forms, and the two are
    /// never reported together.
    #[test]
    fn parses_both_styles_and_never_both() {
        let legacy = parse_command_line(&argv(&[
            "kube-apiserver",
            "--advertise-address=172.18.0.2",
            "--oidc-issuer-url=https://idp.example.com/realms/corp",
            "--oidc-client-id",
            "fathom",
            "--oidc-username-claim=email",
            "--oidc-groups-claim=groups",
            "--oidc-username-prefix=oidc:",
        ]));
        assert_eq!(legacy.style, ApiServerAuthStyle::Legacy);
        assert_eq!(
            legacy.issuer.as_deref(),
            Some("https://idp.example.com/realms/corp")
        );
        assert_eq!(legacy.client_id.as_deref(), Some("fathom"));
        assert_eq!(legacy.username_claim.as_deref(), Some("email"));
        assert_eq!(legacy.groups_claim.as_deref(), Some("groups"));
        assert_eq!(legacy.username_prefix.as_deref(), Some("oidc:"));
        // Absent, not empty: the API server sets no groups prefix here.
        assert_eq!(legacy.groups_prefix, None);
        assert_eq!(legacy.config_path, None);

        let structured = parse_command_line(&argv(&[
            "kube-apiserver",
            "--authentication-config=/etc/kubernetes/pki/authn.yaml",
        ]));
        assert_eq!(structured.style, ApiServerAuthStyle::Structured);
        assert_eq!(
            structured.config_path.as_deref(),
            Some("/etc/kubernetes/pki/authn.yaml")
        );
        // The structured style says nothing about the legacy fields, so none are invented.
        assert_eq!(structured.issuer, None);

        // No OIDC at all is a *known* answer, distinct from "could not tell".
        assert_eq!(
            parse_command_line(&argv(&["kube-apiserver", "--authorization-mode=Node,RBAC"])).style,
            ApiServerAuthStyle::None
        );

        // Kubernetes rejects both at once, so Fathom refuses to pick a winner.
        let both = parse_command_line(&argv(&[
            "kube-apiserver",
            "--authentication-config=/etc/kubernetes/pki/authn.yaml",
            "--oidc-issuer-url=https://idp.example.com/realms/corp",
        ]));
        assert_eq!(both.style, ApiServerAuthStyle::Unknown);
        assert_eq!(both.issuer, None);
    }

    /// T034 (FR-010): a denied or impossible read is a successful command that states the
    /// unknown — never an error, never an assumed pass.
    #[test]
    fn a_denied_read_states_the_unknown() {
        let denied = "pods is forbidden: User \"alice\" cannot list resource \"pods\" in API \
                      group \"\" in the namespace \"kube-system\"";
        let read = read_from_pods(Err(denied.to_owned()), "kind-dev");
        assert_eq!(read.style, ApiServerAuthStyle::Unknown);
        assert!(!read.checked);
        // Verbatim: the operator fixes RBAC from the API server's words, not a paraphrase.
        assert_eq!(read.reason.as_deref(), Some(denied));
        assert_eq!(read.context, "kind-dev");
        assert_eq!(read.issuer, None);
        // An unchecked read can never produce a claim of agreement.
        assert!(compare(&read, &draft()).is_empty());

        // A managed control plane exposes no API server pod at all.
        let managed = read_from_pods(Ok(vec![]), "prod-eks");
        assert_eq!(managed.style, ApiServerAuthStyle::Unknown);
        assert!(!managed.checked);
        assert!(managed.reason.unwrap().contains("managed control plane"));

        // The structured style is readable as a *style* but not as values, because the file it
        // names is on the node's disk and not in the API.
        let structured = read_from_pods(
            Ok(vec![pod(&[
                "kube-apiserver",
                "--authentication-config=/etc/kubernetes/pki/authn.yaml",
            ])]),
            "kind-dev",
        );
        assert_eq!(structured.style, ApiServerAuthStyle::Structured);
        assert!(!structured.checked);
        assert!(
            structured
                .reason
                .unwrap()
                .contains("/etc/kubernetes/pki/authn.yaml")
        );
    }

    /// T035: a successful read reports the flags and the context it came from.
    #[test]
    fn a_successful_read_reports_the_flags_and_context() {
        let read = read_from_pods(
            Ok(vec![pod(&[
                "kube-apiserver",
                "--oidc-issuer-url=https://idp.example.com/realms/corp",
                "--oidc-client-id=fathom",
                "--oidc-username-claim=email",
                "--oidc-groups-claim=groups",
            ])]),
            "kind-dev",
        );
        assert!(read.checked);
        assert_eq!(read.reason, None);
        assert_eq!(read.style, ApiServerAuthStyle::Legacy);
        assert_eq!(read.context, "kind-dev");
        assert_eq!(read.client_id.as_deref(), Some("fathom"));

        // A cluster with no OIDC at all was still successfully read.
        let bare = read_from_pods(Ok(vec![pod(&["kube-apiserver", "--v=2"])]), "kind-dev");
        assert!(bare.checked);
        assert_eq!(bare.style, ApiServerAuthStyle::None);
    }

    /// T036 (FR-011): absent and different are distinct outcomes.
    #[test]
    fn comparison_distinguishes_absent_from_different() {
        let read = read_from_pods(
            Ok(vec![pod(&[
                "kube-apiserver",
                "--oidc-issuer-url=https://old.example.com/realms/corp",
                "--oidc-client-id=fathom",
                "--oidc-username-claim=email",
                "--oidc-groups-claim=groups",
                "--oidc-username-prefix=oidc:",
            ])]),
            "kind-dev",
        );
        let diffs = compare(&read, &draft());
        let by_field = |f: &str| diffs.iter().find(|d| d.field == f).cloned();

        // Different: both sides set it, to different values.
        let issuer = by_field("issuer").expect("issuer differs");
        assert_eq!(
            issuer.cluster.as_deref(),
            Some("https://old.example.com/realms/corp")
        );
        assert_eq!(
            issuer.draft.as_deref(),
            Some("https://idp.example.com/realms/corp")
        );

        // Absent: the cluster sets no groups prefix, which is not the same as setting another
        // one, and the UI must be able to say so.
        let groups_prefix = by_field("groupsPrefix").expect("groups prefix differs");
        assert_eq!(groups_prefix.cluster, None);
        assert_eq!(groups_prefix.draft.as_deref(), Some("oidc:"));

        // Agreeing fields are absent from the list entirely.
        assert!(by_field("clientId").is_none());
        assert!(by_field("usernameClaim").is_none());
        assert!(by_field("usernamePrefix").is_none());

        // A cluster with nothing configured differs in every field the draft sets, and each one
        // is reported as absent on the cluster side.
        let bare = read_from_pods(Ok(vec![pod(&["kube-apiserver", "--v=2"])]), "kind-dev");
        let bare_diffs = compare(&bare, &draft());
        assert_eq!(bare_diffs.len(), 6);
        assert!(bare_diffs.iter().all(|d| d.cluster.is_none()));

        // And an API server already matching the draft produces no differences.
        let matching = read_from_pods(
            Ok(vec![pod(&[
                "kube-apiserver",
                "--oidc-issuer-url=https://idp.example.com/realms/corp",
                "--oidc-client-id=fathom",
                "--oidc-username-claim=email",
                "--oidc-groups-claim=groups",
                "--oidc-username-prefix=oidc:",
                "--oidc-groups-prefix=oidc:",
            ])]),
            "kind-dev",
        );
        assert!(compare(&matching, &draft()).is_empty());
    }

    // -----------------------------------------------------------------------
    // T059: the write's ordering and failure handling
    // -----------------------------------------------------------------------

    struct RecordingNode {
        ops: RefCell<Vec<String>>,
        manifest: RefCell<String>,
    }

    impl RecordingNode {
        fn new(manifest: &str) -> Self {
            Self {
                ops: RefCell::new(Vec::new()),
                manifest: RefCell::new(manifest.to_owned()),
            }
        }
        fn ops(&self) -> Vec<String> {
            self.ops.borrow().clone()
        }
    }

    impl NodeExec for RecordingNode {
        fn run(&self, args: &[&str]) -> Result<String, String> {
            self.ops.borrow_mut().push(args.join(" "));
            if args.first() == Some(&"cat") {
                return Ok(self.manifest.borrow().clone());
            }
            Ok(String::new())
        }
        fn write_file(&self, path: &str, contents: &str) -> Result<(), String> {
            self.ops.borrow_mut().push(format!("write {path}"));
            if path == MANIFEST_PATH {
                *self.manifest.borrow_mut() = contents.to_owned();
            }
            Ok(())
        }
    }

    const MANIFEST: &str = "apiVersion: v1\nkind: Pod\nspec:\n  containers:\n  - command:\n    - kube-apiserver\n    - --advertise-address=172.18.0.2\n    - --authorization-mode=Node,RBAC\n    image: registry.k8s.io/kube-apiserver:v1.34.0\n";

    fn desired(style: ApiServerAuthStyle) -> DesiredAuth {
        DesiredAuth {
            style,
            issuer_url: "https://idp.example.com/realms/corp".into(),
            client_id: "fathom".into(),
            username_claim: "email".into(),
            groups_claim: "groups".into(),
            username_prefix: Some("oidc:".into()),
            groups_prefix: None,
        }
    }

    #[test]
    fn write_backs_up_first_then_mutates_then_waits() {
        let node = RecordingNode::new(MANIFEST);
        let waited = RefCell::new(false);
        apply_write(&node, &desired(ApiServerAuthStyle::Legacy), &|| {
            // By the time readiness is polled the manifest must already be the new one.
            *waited.borrow_mut() = true;
            Ok(())
        })
        .unwrap();
        assert!(*waited.borrow(), "readiness is waited for after the write");

        let ops = node.ops();
        // Backup strictly before the read and the write (FR-021).
        assert_eq!(ops[0], format!("cp -n {MANIFEST_PATH} {BACKUP_PATH}"));
        assert_eq!(ops[1], format!("cat {MANIFEST_PATH}"));
        assert_eq!(ops[2], format!("write {MANIFEST_PATH}"));
        assert_eq!(ops.len(), 3, "a successful write restores nothing: {ops:?}");

        let out = node.manifest.borrow().clone();
        assert!(out.contains("    - --oidc-issuer-url=https://idp.example.com/realms/corp"));
        assert!(out.contains("    - --oidc-username-prefix=oidc:"));
        // An unset prefix is not written as an empty flag.
        assert!(!out.contains("--oidc-groups-prefix"));
        // The flags Fathom did not touch survive, in place.
        assert!(out.contains("    - --authorization-mode=Node,RBAC"));
        assert!(out.contains("image: registry.k8s.io/kube-apiserver:v1.34.0"));
    }

    #[test]
    fn a_control_plane_that_does_not_come_back_is_rolled_back_verbatim() {
        let node = RecordingNode::new(MANIFEST);
        let err = apply_write(&node, &desired(ApiServerAuthStyle::Legacy), &|| {
            Err("the API server did not become ready within 120s: connection refused".to_owned())
        })
        .expect_err("a control plane that never returns is a failure");
        // Verbatim, so the operator sees what actually went wrong, plus what Fathom did about it.
        assert!(
            err.contains("the API server did not become ready within 120s: connection refused")
        );
        assert!(err.contains(BACKUP_PATH));
        let ops = node.ops();
        assert_eq!(
            ops.last().unwrap(),
            &format!("cp {BACKUP_PATH} {MANIFEST_PATH}")
        );
    }

    /// T060: the write follows the cluster's existing style and never produces both.
    #[test]
    fn write_uses_one_style_only() {
        // A cluster already using the legacy flags has them replaced, not duplicated.
        let legacy_existing = MANIFEST.replace(
            "    - --authorization-mode=Node,RBAC",
            "    - --authorization-mode=Node,RBAC\n    - --oidc-issuer-url=https://old.example.com\n    - --oidc-client-id=old",
        );
        let out = rewrite_manifest(&legacy_existing, &desired(ApiServerAuthStyle::Legacy)).unwrap();
        assert_eq!(out.matches("--oidc-issuer-url").count(), 1);
        assert!(!out.contains("https://old.example.com"));
        assert!(!out.contains("--oidc-client-id=old"));
        assert!(!out.contains("--authentication-config"));

        // A cluster using the structured style keeps it, and the legacy flags are not added.
        let structured_existing = MANIFEST.replace(
            "    - --authorization-mode=Node,RBAC",
            "    - --authorization-mode=Node,RBAC\n    - --authentication-config=/etc/kubernetes/pki/authn.yaml",
        );
        let out = rewrite_manifest(
            &structured_existing,
            &desired(ApiServerAuthStyle::Structured),
        )
        .unwrap();
        assert!(!out.contains("--oidc-"));
        assert_eq!(out.matches("--authentication-config=").count(), 1);
        assert!(out.contains(&format!("--authentication-config={AUTH_CONFIG_PATH}")));

        // Structured writes the config file before the manifest that references it.
        let node = RecordingNode::new(&structured_existing);
        apply_write(&node, &desired(ApiServerAuthStyle::Structured), &|| Ok(())).unwrap();
        let ops = node.ops();
        let config_at = ops
            .iter()
            .position(|o| o == &format!("write {AUTH_CONFIG_PATH}"));
        let manifest_at = ops
            .iter()
            .position(|o| o == &format!("write {MANIFEST_PATH}"));
        assert!(config_at < manifest_at, "{ops:?}");

        // A style Fathom could not determine is never written.
        assert!(rewrite_manifest(MANIFEST, &desired(ApiServerAuthStyle::Unknown)).is_err());
        // Nor is a manifest whose command line Fathom cannot find.
        assert!(
            rewrite_manifest("apiVersion: v1\n", &desired(ApiServerAuthStyle::Legacy)).is_err()
        );
    }

    // -----------------------------------------------------------------------
    // T037: the handler's decisions (the kube read itself needs a live cluster)
    // -----------------------------------------------------------------------

    /// FR-022/FR-036: writability comes from the provenance record, never from the context name,
    /// and hosted is never writable at all.
    #[test]
    fn writability_comes_from_provenance_and_is_never_hosted() {
        let provenance = KindProvenanceFile {
            clusters: vec![KindProvenanceRecord {
                cluster: "ours".into(),
                provisioned_at: 1,
                profile: None,
            }],
        };

        let (writable, reason) = writability("kind-ours", &provenance, false);
        assert!(writable);
        assert_eq!(reason, None);

        // Identical-looking context, no record: not writable, and told why.
        let (writable, reason) = writability("kind-theirs", &provenance, false);
        assert!(!writable);
        assert!(reason.unwrap().contains("kind-theirs"));

        // Not a kind context at all.
        assert!(!writability("prod-eks", &provenance, false).0);

        // Hosted never writes a cluster's API server, even one with a record.
        let (writable, reason) = writability("kind-ours", &provenance, true);
        assert!(!writable);
        assert!(reason.unwrap().contains("Hosted"));
    }

    #[test]
    fn comparison_basis_prefers_the_issuer_bound_to_the_context() {
        let other = OidcIssuerConfig {
            id: "other".into(),
            ..draft()
        };
        let config = OidcConfig {
            issuers: vec![draft(), other.clone()],
            contexts: vec![crate::protocol::ContextAuthConfig {
                context_name: "kind-dev".into(),
                mode: crate::protocol::ContextAuthMode::Oidc,
                issuer_id: Some("other".into()),
            }],
            ..OidcConfig::default()
        };
        assert_eq!(
            comparison_basis(&config, "kind-dev").map(|i| i.id),
            Some("other".to_owned())
        );
        // An unbound context with two saved issuers has no basis: a guess would produce
        // differences against an issuer the operator never mentioned.
        assert_eq!(comparison_basis(&config, "kind-two"), None);

        // With exactly one saved issuer there is nothing to guess between.
        let single = OidcConfig {
            issuers: vec![draft()],
            ..OidcConfig::default()
        };
        assert_eq!(
            comparison_basis(&single, "kind-dev").map(|i| i.id),
            Some("corp".to_owned())
        );
        // And with none, no basis at all.
        assert_eq!(comparison_basis(&OidcConfig::default(), "kind-dev"), None);
    }

    #[test]
    fn the_read_is_cached_per_context_for_the_capability_ttl() {
        let cache = ApiServerAuthCache::default();
        let t0 = Instant::now();
        let read = read_from_pods(
            Ok(vec![pod(&[
                "kube-apiserver",
                "--oidc-issuer-url=https://a",
            ])]),
            "kind-dev",
        );
        cache.put(read, t0);

        assert_eq!(
            cache.get("kind-dev", t0).map(|r| r.issuer),
            Some(Some("https://a".to_owned()))
        );
        // Per context: another cluster's read is not answered from this one.
        assert!(cache.get("kind-other", t0).is_none());
        // And the entry expires on the capability cache's own TTL rather than lasting the
        // session, so a flag changed outside Fathom is picked up.
        assert!(
            cache
                .get("kind-dev", t0 + CAPABILITY_TTL + Duration::from_secs(1))
                .is_none()
        );
    }
}

// ---------------------------------------------------------------------------
// T037: the `ReadApiServerAuth` handler
// ---------------------------------------------------------------------------

/// Per-context cache of the API server read.
///
/// Reuses the capability cache's 60 s TTL: the guided path re-enters the comparison step every
/// time the operator steps back and forward, and a static pod's flags do not change underneath
/// them in that window (T037).
#[derive(Default)]
pub(super) struct ApiServerAuthCache {
    entries: std::sync::Mutex<std::collections::HashMap<String, (ApiServerAuthRead, Instant)>>,
}

impl ApiServerAuthCache {
    pub(super) fn get(&self, context: &str, now: Instant) -> Option<ApiServerAuthRead> {
        let entries = self.entries.lock().ok()?;
        let (read, at) = entries.get(context)?;
        (now.saturating_duration_since(*at) < CAPABILITY_TTL).then(|| read.clone())
    }

    /// Drop a context's entry, because its API server has just been changed.
    pub(super) fn invalidate(&self, context: &str) {
        if let Ok(mut entries) = self.entries.lock() {
            entries.remove(context);
        }
    }

    pub(super) fn put(&self, read: ApiServerAuthRead, now: Instant) {
        if let Ok(mut entries) = self.entries.lock() {
            entries.insert(read.context.clone(), (read, now));
        }
    }
}

/// Read the API server's authentication flags as the signed-in user (T035).
async fn read_live(client: kube::Client, context: &str) -> ApiServerAuthRead {
    let api: Api<Pod> = Api::namespaced(client, "kube-system");
    let params = ListParams::default().labels(APISERVER_POD_SELECTOR);
    let pods = api
        .list(&params)
        .await
        .map(|list| list.items)
        // The API server's own words, whatever they are: a 403 explains which verb is missing,
        // and a paraphrase would cost the operator the fix.
        .map_err(|e| e.to_string());
    read_from_pods(pods, context)
}

/// Which saved issuer the cluster is compared against.
///
/// `ReadApiServerAuth` carries no draft (see the report accompanying 045 T037), so the basis is
/// the issuer bound to the active context, else the only saved issuer, else nothing — and
/// nothing means no differences are claimed either way.
fn comparison_basis(config: &OidcConfig, context: &str) -> Option<OidcIssuerConfig> {
    let bound = config
        .contexts
        .iter()
        .find(|c| c.context_name == context)
        .and_then(|c| c.issuer_id.clone());
    if let Some(id) = bound {
        return config.issuers.iter().find(|i| i.id == id).cloned();
    }
    match config.issuers.as_slice() {
        [only] => Some(only.clone()),
        _ => None,
    }
}

/// Whether Fathom may rewrite this API server, and why not when it may not.
///
/// Ownership comes from the provenance record and from nothing else: a cluster created outside
/// Fathom has a `kind-`-prefixed context exactly like one Fathom created (FR-022, FR-036).
#[must_use]
pub fn writability(
    context: &str,
    provenance: &KindProvenanceFile,
    hosted: bool,
) -> (bool, Option<String>) {
    if hosted {
        return (
            false,
            Some(
                "Hosted Fathom never writes a cluster's API server configuration. Apply the \
                 flags below on the control plane yourself."
                    .to_owned(),
            ),
        );
    }
    if provenance.record_for_context(context).is_none() {
        return (
            false,
            Some(format!(
                "Fathom did not provision {context}, so it will not rewrite its API server. \
                 Apply the flags below on the control plane yourself."
            )),
        );
    }
    (true, None)
}

/// `ReadApiServerAuth`: one `ApiServerAuthRead` event for the active context.
pub(super) async fn handle_read_api_server_auth(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
) -> anyhow::Result<()> {
    let context = state.cluster.current_context.clone().ok_or_else(|| {
        anyhow::anyhow!("No cluster is connected, so its API server cannot be read.")
    })?;

    let read = match state.apiserver_auth.get(&context, Instant::now()) {
        Some(read) => read,
        None => {
            // A connection failure is the command's error; a *denied* read is not, and
            // `read_from_pods` has already turned that into a stated unknown.
            let client = state.cluster.client_or_err()?;
            let read = read_live(client, &context).await;
            state.apiserver_auth.put(read.clone(), Instant::now());
            read
        }
    };

    let config = crate::backend::persistence::load_oidc_config();
    let differences = comparison_basis(&config, &context)
        .map(|basis| compare(&read, &basis))
        .unwrap_or_default();
    let (writable, not_writable_reason) = writability(
        &context,
        &crate::backend::persistence::load_kind_provenance(),
        state.identity.is_some(),
    );

    events::emit(
        state.cluster.seq.as_ref(),
        sender,
        Route::cluster(section_keys::access_oidc_config(), Some(context.clone())),
        Correlation::None,
        BackendEvent::ResourceRead(ResourceReadEvent::ApiServerAuthRead(
            ApiServerAuthComparison {
                read,
                differences,
                writable,
                not_writable_reason,
            },
        )),
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// T060/T061: the write, against a real control-plane node
// ---------------------------------------------------------------------------

/// Runs commands in a kind node's container. Only ever constructed for a cluster that has a
/// provenance record.
struct ContainerNode {
    /// `docker` or `podman`.
    binary: &'static str,
    /// The control-plane container, which kind names `<cluster>-control-plane`.
    node: String,
}

impl ContainerNode {
    fn new(runtime: Option<KindRuntimeKind>, cluster: &str) -> Self {
        Self {
            binary: match runtime {
                Some(KindRuntimeKind::Podman) => "podman",
                _ => "docker",
            },
            node: format!("{cluster}-control-plane"),
        }
    }

    fn command(&self, args: &[&str]) -> std::process::Command {
        let mut cmd = std::process::Command::new(self.binary);
        cmd.arg("exec");
        cmd.arg(&self.node);
        cmd.args(args);
        cmd
    }
}

/// Stdout on success; stderr verbatim on failure, because the node's own words are what the
/// operator needs when a control plane will not come back.
fn finish(mut cmd: std::process::Command) -> Result<String, String> {
    let out = cmd.output().map_err(|e| e.to_string())?;
    if out.status.success() {
        return Ok(String::from_utf8_lossy(&out.stdout).into_owned());
    }
    let stderr = String::from_utf8_lossy(&out.stderr).trim().to_owned();
    Err(if stderr.is_empty() {
        format!("{:?} failed with status {}", cmd, out.status)
    } else {
        stderr
    })
}

impl NodeExec for ContainerNode {
    fn run(&self, args: &[&str]) -> Result<String, String> {
        finish(self.command(args))
    }

    fn write_file(&self, path: &str, contents: &str) -> Result<(), String> {
        use std::io::Write;
        // Through stdin rather than an argument: the manifest is multi-line and must not be
        // re-quoted on its way through a shell.
        let mut child = self
            .command(&["sh", "-c", &format!("cat > {path}")])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| e.to_string())?;
        child
            .stdin
            .as_mut()
            .ok_or("could not write to the node")?
            .write_all(contents.as_bytes())
            .map_err(|e| e.to_string())?;
        let out = child.wait_with_output().map_err(|e| e.to_string())?;
        if out.status.success() {
            Ok(())
        } else {
            Err(String::from_utf8_lossy(&out.stderr).trim().to_owned())
        }
    }
}

/// Seconds to allow the API server to come back after the manifest changed.
const READY_TIMEOUT_SECS: u32 = 180;
/// Seconds to wait before polling at all, so a still-running old API server is not mistaken for
/// the new one: the kubelet needs a moment to notice the manifest changed.
const RESTART_GRACE_SECS: u32 = 20;

/// Wait inside the node for the API server to answer `/readyz` again.
///
/// Run on the node rather than through Fathom's own client: the client's connection is being
/// torn down by the very change being made, and the node has the admin kubeconfig the API
/// server's own health check uses.
fn wait_for_control_plane(node: &ContainerNode) -> Result<(), String> {
    let script = format!(
        "sleep {RESTART_GRACE_SECS}; \
         i=0; while [ $i -lt {READY_TIMEOUT_SECS} ]; do \
         kubectl --kubeconfig /etc/kubernetes/admin.conf get --raw /readyz >/dev/null 2>&1 && \
         exit 0; sleep 2; i=$((i+2)); done; \
         echo \"the kube-apiserver did not become ready within {READY_TIMEOUT_SECS}s\" >&2; exit 1"
    );
    node.run(&["sh", "-c", &script]).map(|_| ())
}

/// `WriteApiServerAuth` (045 T061): change a Fathom-provisioned cluster's API server.
///
/// Handled **inline**, never spawned: `audit_summary()` is taken at dispatch
/// (`src/backend.rs`), so a write that completed later would be audited before it happened.
#[allow(clippy::too_many_arguments)]
pub(super) async fn handle_write_api_server_auth(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    cluster: String,
    style: ApiServerAuthStyle,
    issuer_url: String,
    client_id: String,
    username_claim: String,
    groups_claim: String,
    username_prefix: Option<String>,
    groups_prefix: Option<String>,
) -> anyhow::Result<()> {
    let context = crate::backend::kind::context_name_for(&cluster);
    let provenance = crate::backend::persistence::load_kind_provenance();
    let (writable, reason) = writability(&context, &provenance, state.identity.is_some());
    if !writable {
        // A refusal, not a crash: the operator is told why and given the flags to apply by hand.
        anyhow::bail!(
            reason.unwrap_or_else(|| "This cluster is not writable by Fathom.".to_owned())
        );
    }

    let desired = DesiredAuth {
        style,
        issuer_url,
        client_id,
        username_claim,
        groups_claim,
        username_prefix,
        groups_prefix,
    };
    let runtime = crate::backend::kind::detect_prerequisites()
        .await
        .runtime_kind;

    // The node work is blocking (two `docker exec`s and a readiness wait of up to three
    // minutes), but the command must not finish before the cluster has actually changed.
    let node = ContainerNode::new(runtime, &cluster);
    let result = tokio::task::spawn_blocking(move || {
        apply_write(&node, &desired, &|| wait_for_control_plane(&node))
    })
    .await?;

    match result {
        Ok(()) => {
            // The flags just changed, so the cached read is wrong: drop it rather than show the
            // operator a comparison against the configuration from a minute ago.
            state.apiserver_auth.invalidate(&context);
            // Re-read and emit, so the comparison the operator is looking at shows the cluster
            // as it now is rather than as it was before the write.
            if let Ok(client) = state.cluster.client_or_err() {
                let read = read_live(client, &context).await;
                state.apiserver_auth.put(read, Instant::now());
            }
            handle_read_api_server_auth(state, sender).await
        }
        Err(e) => Err(anyhow::anyhow!(e)),
    }
}
