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
use serde::Deserialize;

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
/// Where a replacement file is staged before being renamed into place.
///
/// Deliberately *outside* `/etc/kubernetes/manifests/`: the kubelet parses every file in that
/// directory, and a staging file there would be read as a (broken) static pod of its own.
pub const STAGING_PATH: &str = "/etc/kubernetes/.fathom-write-in-progress";

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
// T033 (FR-008): parsing a structured `AuthenticationConfiguration`
// ---------------------------------------------------------------------------

/// The subset of `apiserver.config.k8s.io` `AuthenticationConfiguration` that bears on the six
/// fields Fathom compares. Unknown keys are ignored: the file legitimately carries more than
/// this (`claimValidationRules`, `userValidationRules`, certificate authorities), and none of it
/// changes which issuer or claims the API server accepts.
#[derive(Debug, Default, Deserialize)]
struct AuthnConfigFile {
    #[serde(default)]
    jwt: Vec<JwtAuthenticator>,
}

#[derive(Debug, Deserialize)]
struct JwtAuthenticator {
    #[serde(default)]
    issuer: JwtIssuer,
    #[serde(default, rename = "claimMappings")]
    claim_mappings: ClaimMappings,
}

#[derive(Debug, Default, Deserialize)]
struct JwtIssuer {
    url: Option<String>,
    #[serde(default)]
    audiences: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
struct ClaimMappings {
    username: Option<ClaimMapping>,
    groups: Option<ClaimMapping>,
}

#[derive(Debug, Default, Deserialize)]
struct ClaimMapping {
    claim: Option<String>,
    prefix: Option<String>,
    expression: Option<String>,
}

/// One claim mapping as a claim name and an optional prefix.
///
/// An empty `prefix: ""` — which is how the structured form spells *no prefix*, and what Fathom
/// itself writes — becomes `None`, because the protocol's `Option<String>` means absent and
/// never the empty string (T006).
fn claim_mapping(field: &str, m: &ClaimMapping) -> Result<(String, Option<String>), String> {
    if let Some(expr) = &m.expression {
        return Err(format!(
            "its {field} is mapped by the CEL expression `{expr}` rather than by a claim name, \
             which has no equivalent among the fields Fathom compares"
        ));
    }
    let claim = m.claim.clone().ok_or_else(|| {
        format!("its {field} claim mapping names neither a claim nor an expression")
    })?;
    Ok((claim, m.prefix.clone().filter(|p| !p.is_empty())))
}

/// What a structured `AuthenticationConfiguration` says, in the same six fields the legacy flags
/// carry.
///
/// Every `Err` here becomes the read's verbatim `reason` with `checked: false` (FR-010): a file
/// that cannot be mapped onto those six fields is reported as an unknown, never guessed at, and
/// never quietly treated as agreement. Each message is phrased to follow "…, but ".
pub fn parse_authentication_config(yaml: &str) -> Result<ParsedAuth, String> {
    let file: AuthnConfigFile = serde_yaml::from_str(yaml).map_err(|e| e.to_string())?;

    let authenticator = match file.jwt.as_slice() {
        [one] => one,
        [] => {
            return Err(
                "it declares no `jwt` authenticator, so it names no issuer to compare".to_owned(),
            );
        }
        many => {
            return Err(format!(
                "it declares {} `jwt` authenticators, and Fathom will not guess which one the \
                 draft should be compared against",
                many.len()
            ));
        }
    };

    let issuer = authenticator
        .issuer
        .url
        .clone()
        .ok_or("its `jwt` authenticator declares no issuer url")?;

    // The API server treats `audiences` as a list, while the legacy flag and the draft each hold
    // one client id. Picking one of several would claim a disagreement that may not exist, so a
    // list is an unknown that names its entries instead.
    let client_id = match authenticator.issuer.audiences.as_slice() {
        [one] => one.clone(),
        [] => return Err(format!("its issuer {issuer} accepts no audiences")),
        many => {
            return Err(format!(
                "its issuer accepts {} audiences ({}), so comparing the draft's client id \
                 against one of them would claim a disagreement that may not exist",
                many.len(),
                many.join(", ")
            ));
        }
    };

    let username = authenticator
        .claim_mappings
        .username
        .as_ref()
        .ok_or("it maps no username claim")?;
    let (username_claim, username_prefix) = claim_mapping("username", username)?;
    let (groups_claim, groups_prefix) = match &authenticator.claim_mappings.groups {
        Some(g) => {
            let (claim, prefix) = claim_mapping("groups", g)?;
            (Some(claim), prefix)
        }
        // Absent, not empty: this API server maps no groups at all.
        None => (None, None),
    };

    Ok(ParsedAuth {
        style: ApiServerAuthStyle::Structured,
        issuer: Some(issuer),
        client_id: Some(client_id),
        username_claim: Some(username_claim),
        groups_claim,
        username_prefix,
        groups_prefix,
        config_path: None,
    })
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
    read_from_pods_with_node(pods, context, None)
}

/// A read that could not be completed: `checked: false` with `reason` verbatim (FR-010).
#[must_use]
pub fn unknown_read(context: &str, style: ApiServerAuthStyle, reason: String) -> ApiServerAuthRead {
    ApiServerAuthRead {
        style,
        issuer: None,
        client_id: None,
        username_claim: None,
        groups_claim: None,
        username_prefix: None,
        groups_prefix: None,
        context: context.to_owned(),
        checked: false,
        reason: Some(reason),
    }
}

/// As [`read_from_pods`], with an optional control-plane node to read a structured
/// `--authentication-config` file from.
///
/// `node` is `Some` only for a cluster whose provenance record says Fathom provisioned it, so a
/// managed or hand-built control plane keeps reporting the stated unknown (FR-022/FR-036). The
/// `cat` failing — file missing, exec denied, cluster gone — is also a stated unknown carrying
/// the node's own words, never a fall back to "the legacy flags are absent, so all is well".
#[must_use]
pub fn read_from_pods_with_node(
    pods: Result<Vec<Pod>, String>,
    context: &str,
    node: Option<&dyn NodeExec>,
) -> ApiServerAuthRead {
    let unknown = |reason: String| unknown_read(context, ApiServerAuthStyle::Unknown, reason);

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
        ApiServerAuthStyle::Structured => {
            let path = parsed
                .config_path
                .clone()
                .unwrap_or_else(|| AUTH_CONFIG_PATH.to_owned());
            read_structured(node, &path, context)
        }
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

/// The structured style's values, from the node when Fathom owns the cluster and as a stated
/// unknown otherwise.
fn read_structured(node: Option<&dyn NodeExec>, path: &str, context: &str) -> ApiServerAuthRead {
    let structured_unknown =
        |reason: String| unknown_read(context, ApiServerAuthStyle::Structured, reason);

    // Without a node the file is simply out of reach: it lives on the control-plane node's
    // filesystem and the Kubernetes API does not serve it.
    let Some(node) = node else {
        return structured_unknown(format!(
            "This API server authenticates with --authentication-config={path}, a file on the \
             control-plane node that the Kubernetes API does not expose. Fathom did not \
             provision this cluster, so it does not read files from its node; the issuer cannot \
             be compared from here, and the legacy --oidc-* flags are not in use."
        ));
    };

    let yaml = match node.run(&["cat", path]) {
        Ok(yaml) => yaml,
        // The node's verbatim words: "No such file or directory" and "No such container" lead
        // the operator to different fixes.
        Err(e) => {
            return structured_unknown(format!(
                "This API server authenticates with --authentication-config={path}, and reading \
                 that file from the control-plane node failed: {e}"
            ));
        }
    };

    match parse_authentication_config(&yaml) {
        Ok(parsed) => ApiServerAuthRead {
            // Still `Structured`, so the comparison can say the legacy flags are not in use.
            style: ApiServerAuthStyle::Structured,
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
        Err(why) => structured_unknown(format!(
            "This API server authenticates with --authentication-config={path}, which Fathom \
             read from the control-plane node, but {why}."
        )),
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
    // Every scalar quoted, for the same reason the flags are: an operator-supplied claim or
    // prefix must not be able to change the shape of the document it lands in.
    y.push_str(&format!(
        "- issuer:\n    url: {:?}\n    audiences:\n    - {:?}\n",
        d.issuer_url, d.client_id
    ));
    y.push_str("  claimMappings:\n");
    let prefix = d.username_prefix.clone().unwrap_or_default();
    y.push_str(&format!(
        "    username:\n      claim: {:?}\n      prefix: {:?}\n",
        d.username_claim, prefix
    ));
    y.push_str(&format!(
        "    groups:\n      claim: {:?}\n      prefix: {:?}\n",
        d.groups_claim,
        d.groups_prefix.clone().unwrap_or_default()
    ));
    y
}

/// The legacy flag lines for a static pod manifest, at `indent`.
/// One kube-apiserver flag as a YAML scalar the kubelet reads back as a *string*.
///
/// A plain scalar is not safe here. `- --oidc-username-prefix=oidc:` ends in a colon, so YAML
/// reads the sequence item as a mapping key rather than a string, and the kubelet rejects the
/// whole manifest:
///
/// ```text
/// couldn't parse as pod(json: cannot unmarshal object into Go struct field
/// Container.spec.containers.command of type string)
/// ```
///
/// The API server then never starts at all — and `oidc:` is the prefix the guided path offers by
/// default, so this was the default configuration. Quoting every flag, not just the ones that
/// currently need it, keeps the question from arising for whatever claim or prefix an operator
/// types next.
fn flag_line(indent: &str, flag: &str, value: &str) -> String {
    let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
    format!("{indent}- \"{flag}={escaped}\"")
}

fn legacy_flag_lines(d: &DesiredAuth, indent: &str) -> Vec<String> {
    let mut lines = vec![
        flag_line(indent, "--oidc-issuer-url", &d.issuer_url),
        flag_line(indent, "--oidc-client-id", &d.client_id),
        flag_line(indent, "--oidc-username-claim", &d.username_claim),
        flag_line(indent, "--oidc-groups-claim", &d.groups_claim),
    ];
    if let Some(p) = &d.username_prefix {
        lines.push(flag_line(indent, "--oidc-username-prefix", p));
    }
    if let Some(p) = &d.groups_prefix {
        lines.push(flag_line(indent, "--oidc-groups-prefix", p));
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
            vec![flag_line(
                indent,
                "--authentication-config",
                AUTH_CONFIG_PATH,
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

/// Replace `path` with `contents` by staging and renaming, never by writing in place.
///
/// The kubelet watches `/etc/kubernetes/manifests` with inotify and re-reads a file the instant
/// it changes. `cat > <manifest>` truncates first and writes second, so the kubelet can read the
/// file while it is empty or half written -- and live, against a real kind node, it did:
///
/// ```text
/// Unable to process watch event ... couldn't parse as pod(Object 'Kind' is missing in 'null')
/// ```
///
/// The cost was not a failed write but a *silently misreported* one. The kubelet did eventually
/// restart the API server with the new flags, yet its mirror pod stayed on the old spec, and the
/// mirror pod is exactly what `read_from_pods` reads. The guided path's comparison step then told
/// the operator the write had not taken effect while the running API server already had it.
///
/// A rename is atomic within a filesystem, so a watcher sees either the whole old file or the
/// whole new one and never something in between.
fn write_atomically(exec: &dyn NodeExec, path: &str, contents: &str) -> Result<(), String> {
    exec.write_file(STAGING_PATH, contents)?;
    // `mv` carries the staging file's mode across, not the target's, and a fresh `cat >` file is
    // created with the node's umask. The manifest it replaces is root-only; match that before
    // the rename rather than quietly loosening it.
    exec.run(&["chmod", "600", STAGING_PATH])?;
    exec.run(&["mv", STAGING_PATH, path]).map(|_| ())
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
        write_atomically(exec, AUTH_CONFIG_PATH, &authentication_config_yaml(d))?;
    }
    write_atomically(exec, MANIFEST_PATH, &rewritten)?;

    // Read the manifest back before trusting it. A write that "succeeded" is not the same as a
    // manifest that holds what we meant: a `docker exec` with no stdin attached truncated this
    // file to zero bytes and still exited 0 (see `ContainerNode::exec`). The readiness wait did
    // eventually catch that, but only by accident of the API server failing to start — a
    // destroyed manifest must be caught as a write failure, not diagnosed as an unhealthy
    // control plane.
    let landed = exec.run(&["cat", MANIFEST_PATH]);
    match landed {
        Ok(ref got) if got.trim() == rewritten.trim() => {}
        other => {
            let detail = match other {
                Ok(got) => format!(
                    "the manifest on the node is not what Fathom wrote ({} bytes written, \
                     {} bytes read back)",
                    rewritten.len(),
                    got.len()
                ),
                Err(e) => format!("the manifest could not be read back after writing it: {e}"),
            };
            return Err(restore(exec, &detail));
        }
    }

    match wait() {
        Ok(()) => Ok(()),
        Err(e) => Err(restore(exec, &e)),
    }
}

/// Put the backed-up manifest back, through the staging file for the same reason the write goes
/// through it: `cp` onto the manifest truncates it in place, and the kubelet reads what it finds.
///
/// This is the path that runs when something has *already* gone wrong, so an in-place copy here
/// is the worst version of the bug: live, it left the kubelet's mirror pod stuck on the
/// configuration being rolled back, which is precisely the state the operator is told was undone.
pub fn restore_manifest(exec: &dyn NodeExec) -> Result<(), String> {
    exec.run(&["cp", BACKUP_PATH, STAGING_PATH])?;
    exec.run(&["chmod", "600", STAGING_PATH])?;
    exec.run(&["mv", STAGING_PATH, MANIFEST_PATH]).map(|_| ())
}

/// Put the backed-up manifest back, and say so in the words of the failure that prompted it.
fn restore(exec: &dyn NodeExec, because: &str) -> String {
    match restore_manifest(exec) {
        Ok(_) => format!(
            "{because}\nThe previous kube-apiserver manifest has been restored from \
             {BACKUP_PATH}."
        ),
        Err(r) => {
            format!(
                "{because}\nRestoring the previous manifest from {BACKUP_PATH} also failed: {r}"
            )
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
        let reason = structured.reason.clone().unwrap();
        assert!(
            reason.contains("/etc/kubernetes/pki/authn.yaml"),
            "{reason}"
        );
        // No node is offered for a cluster Fathom did not provision, and the read says so
        // rather than claiming the file was checked (FR-022).
        assert!(
            reason.contains("did not provision this cluster"),
            "{reason}"
        );
        assert!(compare(&structured, &draft()).is_empty());
    }

    /// A structured file as the API server itself documents it: one `jwt` authenticator with an
    /// issuer url, one audience and both claim mappings.
    const STRUCTURED: &str = "apiVersion: apiserver.config.k8s.io/v1beta1
kind: AuthenticationConfiguration
jwt:
- issuer:
    url: https://idp.example.com/realms/corp
    audiences:
    - fathom
  claimMappings:
    username:
      claim: email
      prefix: \"oidc:\"
    groups:
      claim: groups
      prefix: \"\"
";

    /// US3 scenario 2 (FR-008): for a cluster Fathom provisioned, the structured file is read off
    /// the node and compared like the legacy flags. The style stays `Structured`, so the UI can
    /// still say the legacy flags are not in use.
    #[test]
    fn a_structured_file_is_read_off_a_node_fathom_provisioned() {
        let node =
            RecordingNode::new(MANIFEST).with_file("/etc/kubernetes/pki/authn.yaml", STRUCTURED);
        let read = read_from_pods_with_node(
            Ok(vec![pod(&[
                "kube-apiserver",
                "--authentication-config=/etc/kubernetes/pki/authn.yaml",
            ])]),
            "kind-ours",
            Some(&node),
        );

        assert_eq!(read.style, ApiServerAuthStyle::Structured);
        assert!(read.checked, "{:?}", read.reason);
        assert_eq!(read.reason, None);
        assert_eq!(
            read.issuer.as_deref(),
            Some("https://idp.example.com/realms/corp")
        );
        // The structured `audiences` entry is the same thing the legacy `--oidc-client-id` is.
        assert_eq!(read.client_id.as_deref(), Some("fathom"));
        assert_eq!(read.username_claim.as_deref(), Some("email"));
        assert_eq!(read.groups_claim.as_deref(), Some("groups"));
        assert_eq!(read.username_prefix.as_deref(), Some("oidc:"));
        // An empty prefix is *absent*, never the empty string (T006's rule for the protocol).
        assert_eq!(read.groups_prefix, None);

        // The path read is the one the flag named, not Fathom's own default location.
        assert_eq!(node.ops(), vec!["cat /etc/kubernetes/pki/authn.yaml"]);

        // And it compares like any other read: this draft differs only in the groups prefix.
        let diffs = compare(&read, &draft());
        assert_eq!(diffs.len(), 1, "{diffs:?}");
        assert_eq!(diffs[0].field, "groupsPrefix");
        assert_eq!(diffs[0].cluster, None);
        assert_eq!(diffs[0].draft.as_deref(), Some("oidc:"));
    }

    /// FR-010: a `cat` that fails is still a successful command — the node's verbatim words with
    /// `checked: false`, never a fall back to "the legacy flags are absent, so all is well".
    #[test]
    fn a_structured_file_that_cannot_be_read_states_the_nodes_own_words() {
        let flag = "--authentication-config=/etc/kubernetes/pki/authn.yaml";
        let argv = || Ok(vec![pod(&["kube-apiserver", flag])]);

        // The file the flag names is not on the node at all.
        let missing = RecordingNode::new(MANIFEST);
        let read = read_from_pods_with_node(argv(), "kind-ours", Some(&missing));
        assert_eq!(read.style, ApiServerAuthStyle::Structured);
        assert!(!read.checked);
        let reason = read.reason.clone().unwrap();
        assert!(
            reason.contains("cat: /etc/kubernetes/pki/authn.yaml: No such file or directory"),
            "{reason}"
        );
        assert_eq!(read.issuer, None);
        assert!(compare(&read, &draft()).is_empty());

        // The container is gone, or exec is denied: again the runtime's own words.
        let denied = RecordingNode::new(MANIFEST)
            .failing_cat("Error response from daemon: No such container: ours-control-plane");
        let read = read_from_pods_with_node(argv(), "kind-ours", Some(&denied));
        assert!(!read.checked);
        assert!(
            read.reason
                .unwrap()
                .contains("No such container: ours-control-plane")
        );
    }

    /// A file Fathom cannot map onto the six compared fields is a stated unknown naming why —
    /// not a guess, and not a silent pass.
    #[test]
    fn a_structured_file_fathom_cannot_map_is_a_stated_unknown() {
        let unmappable = |yaml: &str| {
            let node =
                RecordingNode::new(MANIFEST).with_file("/etc/kubernetes/pki/authn.yaml", yaml);
            let read = read_from_pods_with_node(
                Ok(vec![pod(&[
                    "kube-apiserver",
                    "--authentication-config=/etc/kubernetes/pki/authn.yaml",
                ])]),
                "kind-ours",
                Some(&node),
            );
            assert_eq!(read.style, ApiServerAuthStyle::Structured);
            assert!(!read.checked, "{read:?}");
            assert!(compare(&read, &draft()).is_empty());
            read.reason.unwrap()
        };

        // Not YAML at all.
        assert!(!unmappable("{{{").is_empty());

        // No authenticator: there is no issuer to compare against.
        let none = unmappable(
            "apiVersion: apiserver.config.k8s.io/v1beta1\nkind: AuthenticationConfiguration\n",
        );
        assert!(none.contains("jwt"), "{none}");

        // Two authenticators: Fathom will not pick one for the operator.
        let two = unmappable(&STRUCTURED.replace(
            "jwt:\n",
            "jwt:\n- issuer:\n    url: https://other.example.com\n    audiences:\n    - other\n  claimMappings:\n    username:\n      claim: sub\n",
        ));
        assert!(two.contains('2'), "{two}");

        // Two audiences: reporting one of them would claim a disagreement that may not exist.
        let audiences =
            unmappable(&STRUCTURED.replace("    - fathom\n", "    - fathom\n    - fathom-cli\n"));
        assert!(audiences.contains("fathom-cli"), "{audiences}");

        // A CEL expression has no equivalent among the fields Fathom compares.
        let cel = unmappable(&STRUCTURED.replace(
            "      claim: email\n",
            "      expression: \"claims.email\"\n",
        ));
        assert!(cel.contains("claims.email"), "{cel}");
    }

    /// What `apply_write` puts on the node is exactly what the read gets back: the writer and the
    /// parser cannot drift apart without this failing.
    #[test]
    fn what_fathom_writes_is_what_fathom_reads_back() {
        let d = DesiredAuth {
            groups_prefix: Some("idp:".into()),
            ..desired(ApiServerAuthStyle::Structured)
        };
        let parsed = parse_authentication_config(&authentication_config_yaml(&d)).unwrap();
        assert_eq!(parsed.style, ApiServerAuthStyle::Structured);
        assert_eq!(parsed.issuer.as_deref(), Some(d.issuer_url.as_str()));
        assert_eq!(parsed.client_id.as_deref(), Some(d.client_id.as_str()));
        assert_eq!(parsed.username_claim.as_deref(), Some("email"));
        assert_eq!(parsed.groups_claim.as_deref(), Some("groups"));
        assert_eq!(parsed.username_prefix.as_deref(), Some("oidc:"));
        assert_eq!(parsed.groups_prefix.as_deref(), Some("idp:"));

        // And the prefixes Fathom writes as empty come back as absent, not as "".
        let bare = DesiredAuth {
            username_prefix: None,
            groups_prefix: None,
            ..desired(ApiServerAuthStyle::Structured)
        };
        let parsed = parse_authentication_config(&authentication_config_yaml(&bare)).unwrap();
        assert_eq!(parsed.username_prefix, None);
        assert_eq!(parsed.groups_prefix, None);
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

    /// A control-plane node that remembers what was asked of it and serves a small filesystem,
    /// so both the write's ordering and the read's `cat` are assertable without a cluster.
    struct RecordingNode {
        ops: RefCell<Vec<String>>,
        files: RefCell<std::collections::BTreeMap<String, String>>,
        /// What every `cat` fails with, for the exec-denied and cluster-gone cases.
        cat_error: RefCell<Option<String>>,
        /// Reports every write as a success but stores nothing — a real `docker exec` with no
        /// stdin attached behaves exactly like this.
        swallow_writes: RefCell<bool>,
    }

    impl RecordingNode {
        fn new(manifest: &str) -> Self {
            let node = Self {
                ops: RefCell::new(Vec::new()),
                files: RefCell::new(std::collections::BTreeMap::new()),
                cat_error: RefCell::new(None),
                swallow_writes: RefCell::new(false),
            };
            node.put(MANIFEST_PATH, manifest);
            node
        }
        fn put(&self, path: &str, contents: &str) {
            self.files
                .borrow_mut()
                .insert(path.to_owned(), contents.to_owned());
        }
        fn with_file(self, path: &str, contents: &str) -> Self {
            self.put(path, contents);
            self
        }
        fn swallowing_writes(self) -> Self {
            *self.swallow_writes.borrow_mut() = true;
            self
        }
        fn failing_cat(self, error: &str) -> Self {
            *self.cat_error.borrow_mut() = Some(error.to_owned());
            self
        }
        fn ops(&self) -> Vec<String> {
            self.ops.borrow().clone()
        }
        fn manifest(&self) -> String {
            self.files.borrow()[MANIFEST_PATH].clone()
        }
    }

    impl NodeExec for RecordingNode {
        fn run(&self, args: &[&str]) -> Result<String, String> {
            self.ops.borrow_mut().push(args.join(" "));
            if args.first() == Some(&"cat") {
                if let Some(e) = self.cat_error.borrow().clone() {
                    return Err(e);
                }
                let path = args.get(1).copied().unwrap_or_default();
                return self
                    .files
                    .borrow()
                    .get(path)
                    .cloned()
                    // The node's own words for a file that is not there.
                    .ok_or_else(|| format!("cat: {path}: No such file or directory"));
            }
            // `cp`/`mv` move real content around in the fake too, or the read-back check in
            // `apply_write` would be testing the fake's forgetfulness rather than the code.
            // `cp -n` keeps an existing destination, which is the whole point of the backup.
            if let Some(&verb @ ("cp" | "mv")) = args.first() {
                let rest: Vec<&str> = args[1..]
                    .iter()
                    .copied()
                    .filter(|a| !a.starts_with('-'))
                    .collect();
                let no_clobber = args.contains(&"-n");
                if let [from, to] = rest[..] {
                    let existing = self.files.borrow().get(to).cloned();
                    if no_clobber && existing.is_some() {
                        return Ok(String::new());
                    }
                    let content = self
                        .files
                        .borrow()
                        .get(from)
                        .cloned()
                        .ok_or_else(|| format!("{verb}: {from}: No such file or directory"))?;
                    self.put(to, &content);
                    if verb == "mv" {
                        self.files.borrow_mut().remove(from);
                    }
                }
            }
            Ok(String::new())
        }
        fn write_file(&self, path: &str, contents: &str) -> Result<(), String> {
            self.ops.borrow_mut().push(format!("write {path}"));
            if *self.swallow_writes.borrow() {
                self.put(path, "");
            } else {
                self.put(path, contents);
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
        // Staged, mode-matched and renamed into place — never written in place, because the
        // kubelet reads this file the moment it changes (see `write_atomically`).
        assert_eq!(ops[2], format!("write {STAGING_PATH}"));
        assert_eq!(ops[3], format!("chmod 600 {STAGING_PATH}"));
        assert_eq!(ops[4], format!("mv {STAGING_PATH} {MANIFEST_PATH}"));
        // And the manifest is read back before the wait, so a write that landed as something
        // other than what Fathom produced is a write failure rather than an unhealthy cluster.
        assert_eq!(ops[5], format!("cat {MANIFEST_PATH}"));
        assert_eq!(ops.len(), 6, "a successful write restores nothing: {ops:?}");

        let out = node.manifest();
        // Quoted, because `oidc:` would otherwise make the kubelet read the item as a mapping
        // key — see `flag_line`.
        assert!(out.contains(r#"    - "--oidc-issuer-url=https://idp.example.com/realms/corp""#));
        assert!(out.contains(r#"    - "--oidc-username-prefix=oidc:""#));
        // An unset prefix is not written as an empty flag.
        assert!(!out.contains("--oidc-groups-prefix"));
        // The flags Fathom did not touch survive, in place.
        assert!(out.contains("    - --authorization-mode=Node,RBAC"));
        assert!(out.contains("image: registry.k8s.io/kube-apiserver:v1.34.0"));
    }

    /// T076 (FR-021): what Fathom writes must still be a pod the kubelet can parse, for every
    /// prefix an operator may type — above all `oidc:`, the one the guided path suggests.
    ///
    /// Live, the unquoted form made the kubelet reject the manifest outright
    /// (`cannot unmarshal object into Go struct field Container.spec.containers.command of type
    /// string`), so the API server never started and the readiness wait blamed the cluster. No
    /// string assertion caught it; parsing the result back as a `Pod` does.
    #[test]
    fn the_rewritten_manifest_is_still_a_parseable_pod() {
        for prefix in ["oidc:", "", "oidc-", "a: b", "say \"hi\"", "back\\slash"] {
            let mut d = desired(ApiServerAuthStyle::Legacy);
            d.username_prefix = Some(prefix.to_owned());
            d.groups_prefix = Some(prefix.to_owned());
            d.username_claim = prefix.to_owned();

            let out = rewrite_manifest(MANIFEST, &d).expect("rewrite");
            let parsed: Pod = serde_yaml::from_str(&out).unwrap_or_else(|e| {
                panic!("prefix {prefix:?} produced an unparseable pod: {e}\n{out}")
            });
            let argv = parsed
                .spec
                .expect("spec")
                .containers
                .into_iter()
                .next()
                .expect("the kube-apiserver container")
                .command
                .expect("a command");

            // And the flags survive the round trip verbatim, quoting and all.
            assert!(
                argv.contains(&format!("--oidc-username-prefix={prefix}")),
                "prefix {prefix:?} round-trips: {argv:?}"
            );
            assert!(
                argv.contains(&format!("--oidc-username-claim={prefix}")),
                "claim {prefix:?} round-trips: {argv:?}"
            );
            // Reading it straight back is the comparison step's view of the same cluster.
            assert_eq!(
                parse_command_line(&argv).username_prefix.as_deref(),
                Some(prefix)
            );
        }
    }

    /// The structured document gets the same treatment, for the same reason.
    #[test]
    fn the_authentication_config_is_parseable_yaml_for_any_prefix() {
        for prefix in ["oidc:", "", "a: b", "say \"hi\""] {
            let mut d = desired(ApiServerAuthStyle::Structured);
            d.username_prefix = Some(prefix.to_owned());
            d.username_claim = prefix.to_owned();
            let yaml = authentication_config_yaml(&d);
            let v: serde_yaml::Value = serde_yaml::from_str(&yaml).unwrap_or_else(|e| {
                panic!("prefix {prefix:?} produced unparseable YAML: {e}\n{yaml}")
            });
            let mapping = &v["jwt"][0]["claimMappings"]["username"];
            assert_eq!(mapping["prefix"].as_str(), Some(prefix));
            assert_eq!(mapping["claim"].as_str(), Some(prefix));
        }
    }

    /// T076: a write the node reports as a success but which did not land is a failed write, and
    /// the manifest comes back. Found live: `docker exec` without `-i` truncated the real
    /// manifest to zero bytes and exited 0, and only the readiness wait noticed — by which point
    /// the diagnosis on screen blamed the control plane rather than the write.
    #[test]
    fn a_write_that_did_not_land_is_a_failure_and_restores() {
        let node = RecordingNode::new(MANIFEST).swallowing_writes();
        let waited = RefCell::new(false);
        let err = apply_write(&node, &desired(ApiServerAuthStyle::Legacy), &|| {
            *waited.borrow_mut() = true;
            Ok(())
        })
        .expect_err("a manifest that did not land is a failed write");
        assert!(
            !*waited.borrow(),
            "readiness is never waited for on a write that did not land"
        );
        assert!(
            err.contains("not what Fathom wrote"),
            "the failure says the manifest is wrong, not that the cluster is sick: {err}"
        );
        assert!(err.contains(BACKUP_PATH), "and names the restore: {err}");
        let ops = node.ops();
        assert_eq!(
            ops.last().map(String::as_str),
            Some(format!("mv {STAGING_PATH} {MANIFEST_PATH}").as_str()),
            "the last thing it does is rename the backup back into place: {ops:?}"
        );
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
        // Staged and renamed, not copied in place: the rollback must not hand the kubelet a
        // half-written manifest either (see `restore_manifest`).
        assert_eq!(
            &ops[ops.len() - 3..],
            &[
                format!("cp {BACKUP_PATH} {STAGING_PATH}"),
                format!("chmod 600 {STAGING_PATH}"),
                format!("mv {STAGING_PATH} {MANIFEST_PATH}"),
            ]
        );
        // And the manifest really is the original again.
        assert_eq!(node.manifest(), MANIFEST);
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
        // Both are staged and renamed, so the rename is where the destination shows up.
        let config_at = ops
            .iter()
            .position(|o| o == &format!("mv {STAGING_PATH} {AUTH_CONFIG_PATH}"));
        let manifest_at = ops
            .iter()
            .position(|o| o == &format!("mv {STAGING_PATH} {MANIFEST_PATH}"));
        assert!(config_at.is_some() && manifest_at.is_some(), "{ops:?}");
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

    // -----------------------------------------------------------------------
    // T076: live validation against a real cluster.
    //
    // Everything above this line runs against `RecordingNode`, which is why the node side of the
    // write — the readiness grace, the restart race, the rollback, and whether `docker exec cat`
    // can read the manifest at all — was unverified. These two tests are the only thing that
    // exercises it, and they are ignored by default because one of them restarts a control plane.
    // -----------------------------------------------------------------------

    /// Live read (ignored by default):
    /// `FATHOM_APISERVER_LIVE_CONTEXT=kind-fathom-dev cargo test -p fathom --lib live_api_server_read -- --ignored --nocapture`
    #[tokio::test]
    #[ignore = "needs a live cluster"]
    async fn live_api_server_read() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let context =
            std::env::var("FATHOM_APISERVER_LIVE_CONTEXT").expect("FATHOM_APISERVER_LIVE_CONTEXT");
        let cfg = kube::Config::from_kubeconfig(&kube::config::KubeConfigOptions {
            context: Some(context.clone()),
            ..Default::default()
        })
        .await
        .expect("kubeconfig");
        let client = kube::Client::try_from(cfg).expect("client");

        let api: Api<Pod> = Api::namespaced(client, "kube-system");
        let listed = api
            .list(&ListParams::default().labels(APISERVER_POD_SELECTOR))
            .await
            .map(|l| l.items)
            .map_err(|e| e.to_string());
        let read = read_from_pods_with_node(listed, &context, None);
        println!("live read: {read:#?}");

        assert_eq!(read.context, context, "the read names the context it read");
        if read.checked {
            // A cluster with no OIDC configured is a *checked* read whose six fields are absent;
            // that is not the same as a read that could not happen.
            assert!(read.reason.is_none(), "a checked read states no unknown");
        } else {
            assert!(
                read.reason.as_deref().is_some_and(|r| !r.is_empty()),
                "an unchecked read carries the cluster's own words"
            );
            assert!(
                compare(&read, &draft()).is_empty(),
                "an unchecked read can never render as agreement"
            );
        }

        // Ownership is a recorded fact: a cluster Fathom did not provision is not writable, and
        // the refusal names the context rather than guessing from its `kind-` prefix.
        let provenance = crate::backend::persistence::load_kind_provenance();
        let (writable, reason) = writability(&context, &provenance, false);
        println!("writable={writable} reason={reason:?}");
        if provenance.record_for_context(&context).is_none() {
            assert!(!writable, "no provenance record means no write");
            assert!(
                reason.as_deref().is_some_and(|r| r.contains(&context)),
                "the refusal names the context"
            );
        }
    }

    /// Live write, rollback and restore (ignored by default; **restarts the control plane**):
    /// `FATHOM_APISERVER_LIVE_CLUSTER=fathom-prov-045 cargo test -p fathom --lib live_api_server_write -- --ignored --nocapture`
    ///
    /// Run this only against a throwaway cluster. It rewrites the real static pod manifest twice,
    /// so the API server restarts twice, and it leaves the cluster back on its original manifest.
    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "needs a live cluster and restarts its control plane"]
    async fn live_api_server_write() {
        let cluster =
            std::env::var("FATHOM_APISERVER_LIVE_CLUSTER").expect("FATHOM_APISERVER_LIVE_CLUSTER");
        let runtime = crate::backend::kind::detect_prerequisites()
            .await
            .runtime_kind;
        let node = ContainerNode::new(runtime, &cluster);

        // The thing no unit test could show: that `docker|podman exec <node> cat <manifest>`
        // actually reaches a real control plane's filesystem.
        let original = node
            .run(&["cat", MANIFEST_PATH])
            .expect("the control-plane node's manifest is readable");
        assert!(
            original.contains("kube-apiserver"),
            "read a kube-apiserver manifest, got:\n{original}"
        );

        let desired = DesiredAuth {
            style: ApiServerAuthStyle::Legacy,
            issuer_url: "https://idp.example.com/realms/fathom".to_owned(),
            client_id: "fathom".to_owned(),
            username_claim: "preferred_username".to_owned(),
            groups_claim: "groups".to_owned(),
            username_prefix: Some("oidc:".to_owned()),
            groups_prefix: Some("oidc:".to_owned()),
        };

        // 1. The rollback path, with a wait that fails on purpose: the manifest must come back.
        let failed = apply_write(&node, &desired, &|| Err("forced failure".to_owned()))
            .expect_err("a failed wait is a failed write");
        println!("rollback reported: {failed}");
        assert!(
            failed.contains(BACKUP_PATH),
            "the failure names the backup it restored from"
        );
        let after_rollback = node
            .run(&["cat", MANIFEST_PATH])
            .expect("manifest readable");
        assert_eq!(
            after_rollback, original,
            "the rollback restored the manifest byte for byte"
        );

        // 2. The success path, against the real readiness wait. This is the restart race and the
        //    grace period that only a live cluster can prove.
        let marker = marker_flag(&desired);
        apply_write(&node, &desired, &|| wait_for_control_plane(&node, &marker))
            .expect("the control plane comes back with the new flags");
        let written = node
            .run(&["cat", MANIFEST_PATH])
            .expect("manifest readable");
        assert!(
            written.contains("--oidc-issuer-url=https://idp.example.com/realms/fathom"),
            "the flag Fathom wrote is in the manifest:\n{written}"
        );

        // 3. And what the *API server itself* now reports, read back through the Kubernetes API
        //    rather than off the node, which is what the guided path's comparison step shows.
        let _ = rustls::crypto::ring::default_provider().install_default();
        let context = crate::backend::kind::context_name_for(&cluster);
        let cfg = kube::Config::from_kubeconfig(&kube::config::KubeConfigOptions {
            context: Some(context.clone()),
            ..Default::default()
        })
        .await
        .expect("kubeconfig");
        let client = kube::Client::try_from(cfg).expect("client");
        let api: Api<Pod> = Api::namespaced(client, "kube-system");
        let listed = api
            .list(&ListParams::default().labels(APISERVER_POD_SELECTOR))
            .await
            .map(|l| l.items)
            .map_err(|e| e.to_string());
        let read = read_from_pods_with_node(listed, &context, Some(&node));
        println!("live read after the write: {read:#?}");
        assert!(read.checked, "the written config reads back as checked");
        assert_eq!(
            read.issuer.as_deref(),
            Some("https://idp.example.com/realms/fathom")
        );
        assert!(
            compare(&read, &draft()).iter().any(|d| d.field == "issuer"),
            "a draft for a different issuer disagrees on the issuer field"
        );

        // Leave the cluster as it was found.
        restore_manifest(&node).expect("restore the original manifest");
        // A flag the original manifest carries and Fathom never touches, so this waits for the
        // restored API server rather than for anything 045 wrote.
        wait_for_control_plane(&node, "--authorization-mode=Node,RBAC")
            .expect("the control plane comes back on the original");
        let restored = node
            .run(&["cat", MANIFEST_PATH])
            .expect("manifest readable");
        assert_eq!(restored, original, "the cluster is left as it was found");
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
///
/// `node` is `Some` only for a cluster Fathom provisioned, and only then is a structured
/// `--authentication-config` file read off that node; see [`owned_node`].
async fn read_live(
    client: kube::Client,
    context: &str,
    node: Option<ContainerNode>,
) -> ApiServerAuthRead {
    let api: Api<Pod> = Api::namespaced(client, "kube-system");
    let params = ListParams::default().labels(APISERVER_POD_SELECTOR);
    let pods = api
        .list(&params)
        .await
        .map(|list| list.items)
        // The API server's own words, whatever they are: a 403 explains which verb is missing,
        // and a paraphrase would cost the operator the fix.
        .map_err(|e| e.to_string());

    let Some(node) = node else {
        return read_from_pods(pods, context);
    };
    // The node read is a `docker exec`, which blocks; the pod list above was not.
    let ctx = context.to_owned();
    match tokio::task::spawn_blocking(move || read_from_pods_with_node(pods, &ctx, Some(&node)))
        .await
    {
        Ok(read) => read,
        Err(e) => unknown_read(
            context,
            ApiServerAuthStyle::Unknown,
            format!("Reading this API server's configuration did not complete: {e}"),
        ),
    }
}

/// A control-plane node transport for `context`, or `None` when Fathom did not provision that
/// cluster.
///
/// Ownership comes from the provenance record and nothing else, which is why the caller gates on
/// [`writability`] — the same test the write uses, so a hand-built `kind-…` context and a hosted
/// build are both `None` (FR-022/FR-036).
async fn owned_node(provenance: &KindProvenanceFile, context: &str) -> Option<ContainerNode> {
    let cluster = provenance.record_for_context(context)?.cluster.clone();
    let runtime = crate::backend::kind::detect_prerequisites()
        .await
        .runtime_kind;
    Some(ContainerNode::new(runtime, &cluster))
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

/// The API server read for `context`, from the 60 s per-context cache or from the cluster.
///
/// This is the only read any command handler should use: besides the cache it carries the
/// ownership decision that decides whether a structured `--authentication-config` file may be
/// read off the control-plane node (`owned_node`). Calling `read_from_pods` directly skips that
/// and reports a structured cluster as unknown even where Fathom could have read it.
///
/// A *denied* read is not an error — `read_from_pods` has already turned a 403 into a stated
/// unknown (FR-010). Only a missing client is.
pub(super) async fn cached_read(
    state: &BackendState,
    context: &str,
) -> anyhow::Result<ApiServerAuthRead> {
    if let Some(read) = state.apiserver_auth.get(context, Instant::now()) {
        return Ok(read);
    }
    let client = state.cluster.client_or_err()?;
    // The same ownership test the write uses, from the provenance record alone: a hand-built
    // `kind-…` context and a hosted build both get no node (FR-022/FR-036).
    let provenance = crate::backend::persistence::load_kind_provenance();
    let node = match writability(context, &provenance, state.identity.is_some()) {
        (true, _) => owned_node(&provenance, context).await,
        (false, _) => None,
    };
    let read = read_live(client, context, node).await;
    state.apiserver_auth.put(read.clone(), Instant::now());
    Ok(read)
}

/// `ReadApiServerAuth`: one `ApiServerAuthRead` event for the active context.
pub(super) async fn handle_read_api_server_auth(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
) -> anyhow::Result<()> {
    let context = state.cluster.current_context.clone().ok_or_else(|| {
        anyhow::anyhow!("No cluster is connected, so its API server cannot be read.")
    })?;

    let read = cached_read(state, &context).await?;
    let (writable, not_writable_reason) = writability(
        &context,
        &crate::backend::persistence::load_kind_provenance(),
        state.identity.is_some(),
    );

    let config = crate::backend::persistence::load_oidc_config();
    let differences = comparison_basis(&config, &context)
        .map(|basis| compare(&read, &basis))
        .unwrap_or_default();

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
        self.exec(false, args)
    }

    /// `docker exec` *without* `-i` hands the child no stdin at all. `cat > file` then reads EOF
    /// immediately, truncates the file to zero bytes and exits 0 — the manifest is destroyed and
    /// the write reports success. `live_api_server_write` found this against a real node; no fake
    /// `NodeExec` could, because the bug is in how the process is spawned.
    fn exec(&self, interactive: bool, args: &[&str]) -> std::process::Command {
        let mut cmd = std::process::Command::new(self.binary);
        cmd.arg("exec");
        if interactive {
            cmd.arg("-i");
        }
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
            .exec(true, &["sh", "-c", &format!("cat > {path}")])
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
/// The flag whose presence on the live API server proves *this* write took effect.
fn marker_flag(d: &DesiredAuth) -> String {
    match d.style {
        ApiServerAuthStyle::Structured => format!("--authentication-config={AUTH_CONFIG_PATH}"),
        _ => format!("--oidc-issuer-url={}", d.issuer_url),
    }
}

/// Wait until the API server is both answering *and* running the configuration just written.
///
/// Readiness alone is not enough. The grace period exists because the kubelet needs a moment to
/// notice the manifest, and during it the *previous* API server is still up and still answers
/// `/readyz` — so a readiness-only wait can return success while the old configuration is live,
/// and would report a write as applied before it was. Worse, a new configuration that then
/// crash-loops would already have been called a success, past the point where `apply_write` would
/// roll it back.
///
/// So the wait also looks for `marker` in the live API server's own argv. The node container
/// shares its PID namespace with the containers on it, which is why `/proc` can be read for this
/// at all; only processes whose `argv[0]` is `kube-apiserver` are considered, so the matching
/// `grep` cannot find its own command line.
fn wait_for_control_plane(node: &ContainerNode, marker: &str) -> Result<(), String> {
    // Single-quoted into the script, so the `=`, `:` and `/` in a flag are inert.
    let quoted = marker.replace('\'', r"'\''");
    let script = format!(
        "sleep {RESTART_GRACE_SECS}; \
         has_marker() {{ \
         for c in /proc/[0-9]*/cmdline; do \
         first=$(tr '\\0' '\\n' < \"$c\" 2>/dev/null | head -1); \
         case \"$first\" in kube-apiserver) ;; *) continue ;; esac; \
         tr '\\0' '\\n' < \"$c\" 2>/dev/null | grep -qxF -- '{quoted}' && return 0; \
         done; return 1; }}; \
         i=0; ready=0; \
         while [ $i -lt {READY_TIMEOUT_SECS} ]; do \
         if kubectl --kubeconfig /etc/kubernetes/admin.conf get --raw /readyz >/dev/null 2>&1; \
         then ready=1; has_marker && exit 0; fi; \
         sleep 2; i=$((i+2)); done; \
         if [ $ready -eq 1 ]; then \
         echo \"the kube-apiserver is answering but has not picked up the new authentication \
         configuration within {READY_TIMEOUT_SECS}s\" >&2; \
         else \
         echo \"the kube-apiserver did not become ready within {READY_TIMEOUT_SECS}s\" >&2; fi; \
         exit 1"
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
        let marker = marker_flag(&desired);
        apply_write(&node, &desired, &|| wait_for_control_plane(&node, &marker))
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
                // Writable by definition here, so the structured file just written is read back.
                let node = ContainerNode::new(runtime, &cluster);
                let read = read_live(client, &context, Some(node)).await;
                state.apiserver_auth.put(read, Instant::now());
            }
            handle_read_api_server_auth(state, sender).await
        }
        Err(e) => Err(anyhow::anyhow!(e)),
    }
}
