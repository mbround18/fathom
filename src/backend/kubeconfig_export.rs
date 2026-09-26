//! 040-manifest-export: standalone single-context kubeconfig export.
//!
//! Security posture (spec FR-021..FR-032, contracts/protocol.md):
//! - The kubeconfig is handled as untyped YAML, so exec and auth-provider entries are copied
//!   verbatim and nothing is ever run or refreshed (no token is materialized).
//! - Embedded secrets (`token`, `client-key-data`, `password`, auth-provider tokens and
//!   client secret) are left out of the written file unless the operator opts in.
//! - Only a redacted rendering crosses the protocol. The unredacted document exists only in
//!   this module, on its way to a file created with mode 0600.
//! - Logging names the context and the path, never a value.

use std::io::Write as _;
use std::path::{Path, PathBuf};

use base64::Engine as _;
use serde_json::{Map, Value, json};

use crate::protocol::{KubeconfigCredentialKind, KubeconfigExportPreview};

const REDACTED: &str = "<redacted>";

/// Secret keys of `users[].user`.
const USER_SECRET_KEYS: &[&str] = &["token", "client-key-data", "password"];
/// Secret keys of `users[].user.auth-provider.config`.
const AUTH_PROVIDER_SECRET_KEYS: &[&str] =
    &["id-token", "refresh-token", "access-token", "client-secret"];
/// Path-valued user keys that will not resolve on another machine.
const USER_FILE_KEYS: &[&str] = &["client-certificate", "client-key", "tokenFile"];

/// The export for one context, in its three renderings.
#[derive(Debug)]
pub(super) struct BuiltExport {
    /// The document as it will be written with embedded credentials included.
    with_credentials: Value,
    /// The document with every embedded secret field removed.
    without_credentials: Value,
    /// For display: every secret field (and exec env value) replaced with `<redacted>`.
    redacted: Value,
    pub(super) credential: KubeconfigCredentialKind,
    pub(super) embedded_secret_fields: Vec<String>,
    pub(super) file_references: Vec<String>,
    pub(super) warnings: Vec<String>,
}

impl BuiltExport {
    pub(super) fn redacted_yaml(&self) -> String {
        to_yaml(&self.redacted)
    }

    /// The bytes to write. Secret-bearing only when `include_embedded` is true.
    fn file_yaml(&self, include_embedded: bool) -> String {
        if include_embedded {
            to_yaml(&self.with_credentials)
        } else {
            to_yaml(&self.without_credentials)
        }
    }

    pub(super) fn preview(
        &self,
        context: &str,
        suggested_path: &Path,
        source_path: &Path,
    ) -> KubeconfigExportPreview {
        KubeconfigExportPreview {
            context: context.to_owned(),
            redacted_yaml: self.redacted_yaml(),
            credential: self.credential.clone(),
            embedded_secret_fields: self.embedded_secret_fields.clone(),
            file_references: self.file_references.clone(),
            warnings: self.warnings.clone(),
            suggested_path: suggested_path.display().to_string(),
            source_path: source_path.display().to_string(),
        }
    }
}

fn to_yaml(v: &Value) -> String {
    serde_yaml::to_string(v).unwrap_or_default()
}

/// The kubeconfig Fathom reads contexts from, parsed untyped.
pub(super) fn load_source() -> Result<(PathBuf, Value), String> {
    let path = super::kube::kubeconfig_path()
        .ok_or_else(|| "no readable kubeconfig found (KUBECONFIG or ~/.kube/config)".to_owned())?;
    let text = std::fs::read_to_string(&path)
        .map_err(|e| format!("could not read {}: {e}", path.display()))?;
    let value: Value = serde_yaml::from_str(&text)
        .map_err(|e| format!("could not parse {}: {e}", path.display()))?;
    Ok((path, value))
}

fn named<'a>(source: &'a Value, list: &str, name: &str) -> Option<&'a Value> {
    source
        .get(list)?
        .as_array()?
        .iter()
        .find(|e| e.get("name").and_then(Value::as_str) == Some(name))
}

/// Build the standalone document for `context`. `base_dir` resolves relative file paths
/// (kubeconfig paths are relative to the file that holds them).
pub(super) fn build_export(
    source: &Value,
    context: &str,
    base_dir: &Path,
) -> Result<BuiltExport, String> {
    let ctx_entry = named(source, "contexts", context)
        .ok_or_else(|| format!("context {context:?} not found in the kubeconfig"))?;
    let ctx = ctx_entry.get("context").cloned().unwrap_or(json!({}));
    let cluster_name = ctx.get("cluster").and_then(Value::as_str).unwrap_or("");
    let user_name = ctx.get("user").and_then(Value::as_str).unwrap_or("");

    let mut warnings = Vec::new();
    let mut file_references = Vec::new();

    let mut cluster = named(source, "clusters", cluster_name)
        .cloned()
        .ok_or_else(|| format!("cluster {cluster_name:?} of context {context:?} not found"))?;
    inline_ca(&mut cluster, base_dir, &mut file_references, &mut warnings);

    let user = if user_name.is_empty() {
        None
    } else {
        Some(
            named(source, "users", user_name)
                .cloned()
                .ok_or_else(|| format!("user {user_name:?} of context {context:?} not found"))?,
        )
    };

    let user_body = user
        .as_ref()
        .and_then(|u| u.get("user"))
        .cloned()
        .unwrap_or(json!({}));
    let credential = classify_credential(&user_body);
    let embedded_secret_fields = embedded_secret_fields(&user_body);
    for key in USER_FILE_KEYS {
        if let Some(p) = user_body.get(*key).and_then(Value::as_str) {
            file_references.push(format!("{key}: {p}"));
        }
    }
    if let KubeconfigCredentialKind::Exec { command } = &credential {
        warnings.push(format!(
            "Uses the exec plugin {command:?}: it must be installed on the machine that uses this file."
        ));
        if user_body
            .pointer("/exec/env")
            .and_then(Value::as_array)
            .is_some_and(|e| !e.is_empty())
        {
            warnings.push(
                "The exec plugin's env entries are exported as-is; review them before sharing."
                    .to_owned(),
            );
        }
    }
    if !file_references.is_empty() {
        warnings.push(
            "Some entries point at local files and will not work on another machine.".to_owned(),
        );
    }

    let doc = |user_body: Option<Value>| -> Value {
        let mut d = Map::new();
        d.insert("apiVersion".into(), json!("v1"));
        d.insert("kind".into(), json!("Config"));
        d.insert("preferences".into(), json!({}));
        d.insert("clusters".into(), json!([cluster.clone()]));
        let mut ctx_out = ctx_entry.clone();
        if let Some(o) = ctx_out.as_object_mut() {
            o.insert("name".into(), json!(context));
        }
        d.insert("contexts".into(), json!([ctx_out]));
        d.insert("current-context".into(), json!(context));
        let users = match (&user, user_body) {
            (Some(u), Some(body)) => {
                let mut u = u.clone();
                if let Some(o) = u.as_object_mut() {
                    o.insert("user".into(), body);
                }
                json!([u])
            }
            _ => json!([]),
        };
        d.insert("users".into(), users);
        Value::Object(d)
    };

    let with_credentials = doc(user.as_ref().map(|_| user_body.clone()));
    let mut stripped = user_body.clone();
    strip_secrets(&mut stripped);
    let without_credentials = doc(user.as_ref().map(|_| stripped));
    let mut redacted_body = user_body.clone();
    redact_secrets(&mut redacted_body);
    let redacted = doc(user.as_ref().map(|_| redacted_body));

    if !embedded_secret_fields.is_empty() {
        warnings.push(format!(
            "Embedded credentials ({}) are left out unless you choose to include them.",
            embedded_secret_fields.join(", ")
        ));
    }

    Ok(BuiltExport {
        with_credentials,
        without_credentials,
        redacted,
        credential,
        embedded_secret_fields,
        file_references,
        warnings,
    })
}

/// Replace a `certificate-authority` path with inline data when the file is readable.
fn inline_ca(
    cluster: &mut Value,
    base_dir: &Path,
    file_references: &mut Vec<String>,
    warnings: &mut Vec<String>,
) {
    let Some(body) = cluster.get_mut("cluster").and_then(Value::as_object_mut) else {
        return;
    };
    let Some(ca_path) = body
        .get("certificate-authority")
        .and_then(Value::as_str)
        .map(str::to_owned)
    else {
        return;
    };
    let resolved = resolve(base_dir, &ca_path);
    match std::fs::read(&resolved) {
        Ok(bytes) => {
            body.remove("certificate-authority");
            body.insert(
                "certificate-authority-data".into(),
                json!(base64::engine::general_purpose::STANDARD.encode(bytes)),
            );
        }
        Err(_) => {
            file_references.push(format!("certificate-authority: {ca_path}"));
            warnings.push(format!(
                "The cluster CA file {ca_path} could not be read, so it was kept as a path."
            ));
        }
    }
}

fn resolve(base_dir: &Path, p: &str) -> PathBuf {
    let path = expand_tilde(p);
    if path.is_absolute() {
        path
    } else {
        base_dir.join(path)
    }
}

/// Credential type, by precedence: exec, auth-provider, token, tokenFile, client
/// certificate, basic, none (research R6).
pub(super) fn classify_credential(user: &Value) -> KubeconfigCredentialKind {
    let has = |k: &str| user.get(k).is_some_and(|v| !v.is_null());
    if let Some(exec) = user.get("exec").filter(|v| !v.is_null()) {
        let command = exec
            .get("command")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        return KubeconfigCredentialKind::Exec { command };
    }
    if let Some(ap) = user.get("auth-provider").filter(|v| !v.is_null()) {
        let name = ap
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        return KubeconfigCredentialKind::AuthProvider { name };
    }
    if has("token") {
        return KubeconfigCredentialKind::Token;
    }
    if has("tokenFile") {
        return KubeconfigCredentialKind::TokenFile;
    }
    if has("client-certificate-data") || has("client-certificate") || has("client-key-data") {
        return KubeconfigCredentialKind::ClientCertificate;
    }
    if has("username") || has("password") {
        return KubeconfigCredentialKind::Basic;
    }
    KubeconfigCredentialKind::None
}

/// Names of the secret fields present in a user entry.
pub(super) fn embedded_secret_fields(user: &Value) -> Vec<String> {
    let mut out: Vec<String> = USER_SECRET_KEYS
        .iter()
        .filter(|k| user.get(**k).is_some_and(|v| !v.is_null()))
        .map(|k| (*k).to_owned())
        .collect();
    if let Some(cfg) = user.pointer("/auth-provider/config") {
        for k in AUTH_PROVIDER_SECRET_KEYS {
            if cfg.get(*k).is_some_and(|v| !v.is_null()) {
                out.push(format!("auth-provider.config.{k}"));
            }
        }
    }
    out
}

fn strip_secrets(user: &mut Value) {
    if let Some(o) = user.as_object_mut() {
        for k in USER_SECRET_KEYS {
            o.remove(*k);
        }
    }
    if let Some(cfg) = user
        .pointer_mut("/auth-provider/config")
        .and_then(Value::as_object_mut)
    {
        for k in AUTH_PROVIDER_SECRET_KEYS {
            cfg.remove(*k);
        }
    }
}

fn redact_secrets(user: &mut Value) {
    if let Some(o) = user.as_object_mut() {
        for k in USER_SECRET_KEYS {
            if let Some(v) = o.get_mut(*k) {
                *v = json!(REDACTED);
            }
        }
    }
    if let Some(cfg) = user
        .pointer_mut("/auth-provider/config")
        .and_then(Value::as_object_mut)
    {
        for k in AUTH_PROVIDER_SECRET_KEYS {
            if let Some(v) = cfg.get_mut(*k) {
                *v = json!(REDACTED);
            }
        }
    }
    // Exec env values are written as-is but may hold secrets: never display them.
    if let Some(env) = user.pointer_mut("/exec/env").and_then(Value::as_array_mut) {
        for e in env {
            if let Some(v) = e.get_mut("value") {
                *v = json!(REDACTED);
            }
        }
    }
}

fn expand_tilde(p: &str) -> PathBuf {
    if p == "~" {
        return dirs::home_dir().unwrap_or_else(|| PathBuf::from(p));
    }
    if let Some(rest) = p.strip_prefix("~/")
        && let Some(home) = dirs::home_dir()
    {
        return home.join(rest);
    }
    PathBuf::from(p)
}

/// `~/.kube/fathom-<context>.yaml`, with characters unsafe in a file name replaced.
pub(super) fn suggested_path(context: &str) -> PathBuf {
    let safe: String = context
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '-'
            }
        })
        .collect();
    expand_tilde("~/.kube").join(format!("fathom-{safe}.yaml"))
}

/// Validate `target` and write `built` to it with owner-only permissions. Returns the
/// absolute path written.
pub(super) fn write_export(
    built: &BuiltExport,
    target: &str,
    source_path: &Path,
    include_embedded: bool,
    overwrite: bool,
) -> Result<PathBuf, String> {
    let path = expand_tilde(target.trim());
    if !path.is_absolute() {
        return Err(format!(
            "the target path must be absolute (or start with ~/): {target}"
        ));
    }
    if !path.parent().is_some_and(Path::is_dir) {
        return Err(format!("the folder for {} does not exist", path.display()));
    }
    if same_file(&path, source_path) {
        return Err(
            "refusing to overwrite the kubeconfig Fathom loads contexts from; choose another path"
                .to_owned(),
        );
    }
    let existing = std::fs::symlink_metadata(&path).ok();
    if let Some(meta) = &existing {
        if meta.file_type().is_symlink() || meta.is_dir() {
            return Err(format!(
                "{} is a directory or a symlink; choose a regular file path",
                path.display()
            ));
        }
        if !overwrite {
            return Err(format!(
                "{} already exists; tick Overwrite to replace it",
                path.display()
            ));
        }
    }

    let mut opts = std::fs::OpenOptions::new();
    opts.write(true);
    if overwrite {
        opts.create(true).truncate(true);
    } else {
        opts.create_new(true);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        opts.mode(0o600);
    }
    let mut file = opts
        .open(&path)
        .map_err(|e| format!("could not create {}: {e}", path.display()))?;
    // An overwritten file keeps its old mode on open: tighten it before any byte is written.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))
            .map_err(|e| format!("could not restrict {}: {e}", path.display()))?;
    }
    file.write_all(built.file_yaml(include_embedded).as_bytes())
        .and_then(|()| file.sync_all())
        .map_err(|e| format!("could not write {}: {e}", path.display()))?;
    Ok(path)
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(x), Ok(y)) => x == y,
        _ => a == b,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRETS: &[&str] = &[
        "tok-SECRET-123",
        "a2V5LVNFQ1JFVA==",
        "pw-SECRET",
        "idtok-SECRET",
        "refresh-SECRET",
        "client-SECRET",
        "env-SECRET",
    ];

    fn source() -> Value {
        serde_yaml::from_str(
            r#"
apiVersion: v1
kind: Config
current-context: tok
clusters:
- name: c1
  cluster:
    server: https://c1.example:6443
    certificate-authority-data: Q0E=
- name: c2
  cluster:
    server: https://c2.example:6443
    certificate-authority: ca.crt
contexts:
- name: tok
  context: {cluster: c1, user: u-token, namespace: web}
- name: exec
  context: {cluster: c1, user: u-exec}
- name: cert
  context: {cluster: c2, user: u-cert}
- name: oidc
  context: {cluster: c1, user: u-oidc}
- name: basic
  context: {cluster: c1, user: u-basic}
- name: other
  context: {cluster: c2, user: u-token}
users:
- name: u-token
  user: {token: tok-SECRET-123}
- name: u-exec
  user:
    exec:
      apiVersion: client.authentication.k8s.io/v1beta1
      command: aws
      args: [eks, get-token, --cluster-name, prod]
      env:
      - {name: AWS_PROFILE, value: env-SECRET}
- name: u-cert
  user:
    client-certificate-data: Q0VSVA==
    client-key-data: a2V5LVNFQ1JFVA==
- name: u-oidc
  user:
    auth-provider:
      name: oidc
      config:
        idp-issuer-url: https://issuer
        client-id: fathom
        client-secret: client-SECRET
        id-token: idtok-SECRET
        refresh-token: refresh-SECRET
- name: u-basic
  user: {username: admin, password: pw-SECRET}
"#,
        )
        .unwrap()
    }

    fn build(ctx: &str) -> BuiltExport {
        build_export(&source(), ctx, Path::new("/nonexistent")).unwrap()
    }

    #[test]
    fn standalone_document_has_only_the_chosen_context() {
        let b = build("tok");
        let v = &b.with_credentials;
        assert_eq!(v["current-context"], "tok");
        assert_eq!(v["contexts"].as_array().unwrap().len(), 1);
        assert_eq!(v["clusters"].as_array().unwrap().len(), 1);
        assert_eq!(v["clusters"][0]["name"], "c1");
        assert_eq!(v["users"][0]["name"], "u-token");
        assert_eq!(v["contexts"][0]["context"]["namespace"], "web");
        assert!(!to_yaml(v).contains("c2.example"));
    }

    #[test]
    fn classifies_every_credential_kind() {
        assert_eq!(build("tok").credential, KubeconfigCredentialKind::Token);
        assert_eq!(
            build("exec").credential,
            KubeconfigCredentialKind::Exec {
                command: "aws".into()
            }
        );
        assert_eq!(
            build("cert").credential,
            KubeconfigCredentialKind::ClientCertificate
        );
        assert_eq!(
            build("oidc").credential,
            KubeconfigCredentialKind::AuthProvider {
                name: "oidc".into()
            }
        );
        assert_eq!(build("basic").credential, KubeconfigCredentialKind::Basic);
        assert_eq!(
            classify_credential(&json!({"tokenFile": "/t"})),
            KubeconfigCredentialKind::TokenFile
        );
        assert_eq!(
            classify_credential(&json!({})),
            KubeconfigCredentialKind::None
        );
    }

    #[test]
    fn preview_never_contains_a_secret() {
        for ctx in ["tok", "exec", "cert", "oidc", "basic"] {
            let b = build(ctx);
            let preview =
                serde_json::to_string(&b.preview(ctx, Path::new("/tmp/x"), Path::new("/src")))
                    .unwrap();
            for s in SECRETS {
                assert!(!preview.contains(s), "{ctx}: preview leaked {s}");
            }
        }
    }

    #[test]
    fn embedded_fields_are_reported_and_stripped_by_default() {
        let b = build("oidc");
        assert_eq!(
            b.embedded_secret_fields,
            [
                "auth-provider.config.id-token",
                "auth-provider.config.refresh-token",
                "auth-provider.config.client-secret"
            ]
        );
        let plain = b.file_yaml(false);
        for s in SECRETS {
            assert!(!plain.contains(s), "stripped file leaked {s}");
        }
        // Non-secret auth-provider settings are kept as-is.
        assert!(plain.contains("idp-issuer-url"));
        assert!(plain.contains("client-id: fathom"));
        let full = b.file_yaml(true);
        assert!(full.contains("idtok-SECRET"));

        let cert = build("cert");
        assert_eq!(cert.embedded_secret_fields, ["client-key-data"]);
        assert!(!cert.file_yaml(false).contains("a2V5LVNFQ1JFVA=="));
        assert!(cert.file_yaml(false).contains("client-certificate-data"));
        assert_eq!(build("basic").embedded_secret_fields, ["password"]);
        assert_eq!(build("tok").embedded_secret_fields, ["token"]);
    }

    #[test]
    fn exec_is_exported_verbatim() {
        let b = build("exec");
        assert!(b.embedded_secret_fields.is_empty());
        let file: Value = serde_yaml::from_str(&b.file_yaml(false)).unwrap();
        let src = source();
        let original = named(&src, "users", "u-exec").unwrap();
        assert_eq!(&file["users"][0], original);
        // ...but the env value is not displayed.
        assert!(!b.redacted_yaml().contains("env-SECRET"));
        assert!(b.redacted_yaml().contains("get-token"));
    }

    #[test]
    fn ca_path_is_inlined_when_readable_and_listed_otherwise() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("ca.crt"), b"CA-BYTES").unwrap();
        let b = build_export(&source(), "cert", dir.path()).unwrap();
        let cluster = &b.with_credentials["clusters"][0]["cluster"];
        assert!(cluster.get("certificate-authority").is_none());
        assert_eq!(
            cluster["certificate-authority-data"],
            base64::engine::general_purpose::STANDARD.encode(b"CA-BYTES")
        );

        let missing = build("cert");
        assert!(
            missing
                .file_references
                .iter()
                .any(|r| r == "certificate-authority: ca.crt")
        );
    }

    #[test]
    fn unknown_context_is_an_error() {
        assert!(build_export(&source(), "nope", Path::new("/")).is_err());
    }

    #[test]
    fn suggested_path_is_safe() {
        let p = suggested_path("arn:aws:eks:us-east-1:1:cluster/prod");
        let name = p.file_name().unwrap().to_str().unwrap();
        assert_eq!(name, "fathom-arn-aws-eks-us-east-1-1-cluster-prod.yaml");
    }

    #[cfg(unix)]
    fn mode(p: &Path) -> u32 {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::metadata(p).unwrap().permissions().mode() & 0o777
    }

    #[test]
    fn writes_owner_only_and_refuses_unsafe_targets() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("config");
        std::fs::write(&src, "x").unwrap();
        let b = build("tok");
        let target = dir.path().join("out.yaml");
        let t = target.to_str().unwrap();

        let written = write_export(&b, t, &src, false, false).unwrap();
        assert_eq!(written, target);
        let body = std::fs::read_to_string(&target).unwrap();
        assert!(!body.contains("tok-SECRET-123"));
        assert!(body.contains("current-context: tok"));
        #[cfg(unix)]
        assert_eq!(mode(&target), 0o600);

        // Exists, no overwrite.
        assert!(
            write_export(&b, t, &src, false, false)
                .unwrap_err()
                .contains("already exists")
        );

        // Overwrite a world-readable file: ends 0600, with credentials when asked.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o644)).unwrap();
        }
        write_export(&b, t, &src, true, true).unwrap();
        assert!(
            std::fs::read_to_string(&target)
                .unwrap()
                .contains("tok-SECRET-123")
        );
        #[cfg(unix)]
        assert_eq!(mode(&target), 0o600);

        // The source kubeconfig, relative paths and missing folders are refused.
        assert!(
            write_export(&b, src.to_str().unwrap(), &src, false, true)
                .unwrap_err()
                .contains("refusing")
        );
        assert!(
            write_export(&b, "relative.yaml", &src, false, false)
                .unwrap_err()
                .contains("absolute")
        );
        let missing = dir.path().join("nope").join("x.yaml");
        assert!(
            write_export(&b, missing.to_str().unwrap(), &src, false, false)
                .unwrap_err()
                .contains("does not exist")
        );
        assert!(
            write_export(&b, dir.path().to_str().unwrap(), &src, false, true)
                .unwrap_err()
                .contains("directory")
        );
    }
}
