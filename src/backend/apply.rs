//! `ApplyResourceYaml` support: parse+validate the operator's edited YAML against the
//! command's own identity fields (FR-005), then apply it to the cluster with optimistic
//! concurrency via `expected_resource_version`.
//!
//! Dispatch is generic over built-in and CRD kinds alike: rather than maintaining a
//! kind-name -> typed-API table, we discover the `ApiResource` for the command's
//! `apiVersion`/`kind` via `kube::discovery::oneshot::pinned_kind` and apply through
//! `Api::<DynamicObject>::replace()`. This covers every kind the cluster knows about
//! (built-in or CRD) without a manually-maintained dispatch table, at the cost of one
//! extra discovery round-trip per apply.

use kube::api::{Api, DynamicObject, PostParams};
use kube::core::gvk::GroupVersionKind;
use kube::discovery;
use kube::{Client, ResourceExt};

/// Outcome of a failed apply, classified so the frontend can render three distinct,
/// non-conflated messages (FR-004/FR-007, and the NEW deleted-mid-edit case).
#[derive(Debug)]
pub enum ApplyError {
    /// 409 from the API server: `expected_resource_version` is stale.
    Conflict(String),
    /// 404 from the API server: the object was deleted after the editor opened.
    NotFound(String),
    /// Any other failure (validation rejected by the cluster, permission denied,
    /// malformed identity, discovery failure, ...).
    Other(anyhow::Error),
}

impl std::fmt::Display for ApplyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            // Prefixes are the wire contract the frontend classifies on; keep them stable.
            ApplyError::Conflict(msg) => write!(f, "conflict: {msg}"),
            ApplyError::NotFound(msg) => write!(f, "not-found: {msg}"),
            ApplyError::Other(err) => write!(f, "{err:#}"),
        }
    }
}

impl std::error::Error for ApplyError {}

/// Parse `yaml` and check its `apiVersion`/`kind`/`metadata.namespace`/`metadata.name`
/// match the command's own fields, per FR-005 ("apply targets the one resource that was
/// opened, not general resource creation"). Returns the parsed object on success.
pub fn parse_and_validate_yaml(
    yaml: &str,
    api_version: &str,
    kind: &str,
    namespace: Option<&str>,
    name: &str,
) -> Result<serde_json::Value, String> {
    let value: serde_json::Value = serde_yaml::from_str(yaml)
        .map_err(|e| format!("malformed YAML: could not parse the edited content: {e}"))?;

    if !value.is_object() {
        return Err("malformed YAML: the edited content is not a Kubernetes object".to_owned());
    }

    let got_api_version = value.get("apiVersion").and_then(|v| v.as_str());
    let got_kind = value.get("kind").and_then(|v| v.as_str());
    let metadata = value.get("metadata");
    let got_name = metadata
        .and_then(|m| m.get("name"))
        .and_then(|v| v.as_str());
    let got_namespace = metadata
        .and_then(|m| m.get("namespace"))
        .and_then(|v| v.as_str());

    let mut mismatches = Vec::new();
    if got_api_version != Some(api_version) {
        mismatches.push(format!(
            "apiVersion changed from {api_version:?} to {:?}",
            got_api_version.unwrap_or("<missing>")
        ));
    }
    if got_kind != Some(kind) {
        mismatches.push(format!(
            "kind changed from {kind:?} to {:?}",
            got_kind.unwrap_or("<missing>")
        ));
    }
    if got_name != Some(name) {
        mismatches.push(format!(
            "metadata.name changed from {name:?} to {:?}",
            got_name.unwrap_or("<missing>")
        ));
    }
    if got_namespace != namespace {
        mismatches.push(format!(
            "metadata.namespace changed from {:?} to {:?}",
            namespace.unwrap_or("<none>"),
            got_namespace.unwrap_or("<none>")
        ));
    }

    if !mismatches.is_empty() {
        return Err(format!(
            "identity fields cannot change (kind, apiVersion, metadata.name, metadata.namespace \
             target the one resource that was opened): {}",
            mismatches.join("; ")
        ));
    }

    Ok(value)
}

/// Apply `object` (already identity-validated by [`parse_and_validate_yaml`]) to the
/// cluster, using `expected_resource_version` for optimistic concurrency. Discovers the
/// resource's `ApiResource` dynamically so both built-in and CRD kinds are handled the
/// same way.
pub async fn apply_resource(
    client: &Client,
    api_version: &str,
    kind: &str,
    namespace: Option<&str>,
    name: &str,
    mut object: serde_json::Value,
    expected_resource_version: &str,
) -> Result<String, ApplyError> {
    if let Some(meta) = object.get_mut("metadata").and_then(|m| m.as_object_mut()) {
        meta.insert(
            "resourceVersion".to_owned(),
            serde_json::Value::String(expected_resource_version.to_owned()),
        );
    }

    let (group, version) = split_api_version(api_version);
    let gvk = GroupVersionKind::gvk(&group, &version, kind);
    let (api_resource, _caps) = discovery::oneshot::pinned_kind(client, &gvk)
        .await
        .map_err(|e| {
            ApplyError::Other(anyhow::anyhow!(
                "could not discover the API for {kind} ({api_version}): {e}"
            ))
        })?;

    let api: Api<DynamicObject> = match namespace {
        Some(ns) => Api::namespaced_with(client.clone(), ns, &api_resource),
        None => Api::all_with(client.clone(), &api_resource),
    };

    let dyn_obj: DynamicObject = serde_json::from_value(object)
        .map_err(|e| ApplyError::Other(anyhow::anyhow!("invalid object: {e}")))?;

    match api.replace(name, &PostParams::default(), &dyn_obj).await {
        Ok(applied) => Ok(applied.resource_version().unwrap_or_default()),
        Err(kube::Error::Api(resp)) if resp.code == 409 => Err(ApplyError::Conflict(resp.message)),
        Err(kube::Error::Api(resp)) if resp.code == 404 => Err(ApplyError::NotFound(resp.message)),
        Err(e) => Err(ApplyError::Other(e.into())),
    }
}

/// Create-only sibling of [`apply_resource`] (011-rbac-oidc-administration): `.create()`s a
/// brand-new object instead of `.replace()`-ing an existing one. `apply_resource`'s own
/// behavior is intentionally untouched by this function's existence — every other kind's
/// YAML-edit tab keeps going through `apply_resource` exactly as before. Object identity is
/// still validated by the caller via [`parse_and_validate_yaml`] first, same as apply.
pub async fn create_resource(
    client: &Client,
    api_version: &str,
    kind: &str,
    namespace: Option<&str>,
    object: serde_json::Value,
) -> Result<String, ApplyError> {
    let (group, version) = split_api_version(api_version);
    let gvk = GroupVersionKind::gvk(&group, &version, kind);
    let (api_resource, _caps) = discovery::oneshot::pinned_kind(client, &gvk)
        .await
        .map_err(|e| {
            ApplyError::Other(anyhow::anyhow!(
                "could not discover the API for {kind} ({api_version}): {e}"
            ))
        })?;

    let api: Api<DynamicObject> = match namespace {
        Some(ns) => Api::namespaced_with(client.clone(), ns, &api_resource),
        None => Api::all_with(client.clone(), &api_resource),
    };

    let dyn_obj: DynamicObject = serde_json::from_value(object)
        .map_err(|e| ApplyError::Other(anyhow::anyhow!("invalid object: {e}")))?;

    match api.create(&PostParams::default(), &dyn_obj).await {
        Ok(created) => Ok(created.resource_version().unwrap_or_default()),
        // 409 here means "already exists" (a different name collision), not a stale
        // resourceVersion — same wire prefix as apply's conflict since the frontend's
        // classifyApplyError() only branches on the prefix, and "conflict" reads correctly
        // for both cases ("something about doing this right now failed, re-check and retry").
        Err(kube::Error::Api(resp)) if resp.code == 409 => Err(ApplyError::Conflict(resp.message)),
        Err(e) => Err(ApplyError::Other(e.into())),
    }
}

/// Announce a successful apply/create to every client: the one `StreamEvent::ResourceApplied`
/// emission in the backend.
///
/// Extracted (015-mcp-agent-exposure, T037) so the UI-driven `ApplyResourceYaml` /
/// `CreateResourceYaml` handlers in `crate::backend::cmd` and the MCP `apply_resource_yaml` tool
/// in `crate::backend::mcp::tools_mutate` emit the *same* event through the *same* call, rather
/// than each constructing its own. An operator watching Fathom's own UI therefore sees an
/// external agent's apply exactly as they see their own.
pub(super) fn emit_resource_applied(
    seq: &std::sync::atomic::AtomicU64,
    sender: &crossbeam_channel::Sender<crate::protocol::BackendEventEnvelope>,
    resource_version: String,
) {
    use crate::protocol::{BackendEvent, Correlation, Route, StreamEvent, section_keys};

    crate::backend::events::emit(
        seq,
        sender,
        Route::global(section_keys::system()),
        Correlation::None,
        BackendEvent::Stream(StreamEvent::ResourceApplied {
            request_id: crate::backend::events::current_request_id_string(),
            resource_version,
        }),
    );
}

/// Split `"apps/v1"` into `("apps", "v1")`, and `"v1"` into `("", "v1")` (the core group).
/// Discover the `ApiResource` (group/plural) for `api_version`/`kind`. Shared with the
/// 018 access pre-check so the check and the real mutation agree on the resource.
pub(super) async fn resolve_api_resource(
    client: &Client,
    api_version: &str,
    kind: &str,
) -> anyhow::Result<kube::core::ApiResource> {
    let (group, version) = split_api_version(api_version);
    let gvk = GroupVersionKind::gvk(&group, &version, kind);
    let (api_resource, _caps) = discovery::oneshot::pinned_kind(client, &gvk)
        .await
        .map_err(|e| {
            anyhow::anyhow!("could not discover the API for {kind} ({api_version}): {e}")
        })?;
    Ok(api_resource)
}

fn split_api_version(api_version: &str) -> (String, String) {
    match api_version.split_once('/') {
        Some((group, version)) => (group.to_owned(), version.to_owned()),
        None => (String::new(), api_version.to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_matching_identity() {
        let yaml = "apiVersion: apps/v1\nkind: Deployment\nmetadata:\n  name: web\n  namespace: default\nspec:\n  replicas: 3\n";
        let value = parse_and_validate_yaml(yaml, "apps/v1", "Deployment", Some("default"), "web")
            .expect("should parse and validate");
        assert_eq!(value["spec"]["replicas"], 3);
    }

    #[test]
    fn accepts_cluster_scoped_object_with_no_namespace() {
        let yaml = "apiVersion: v1\nkind: PersistentVolume\nmetadata:\n  name: pv-1\nspec: {}\n";
        parse_and_validate_yaml(yaml, "v1", "PersistentVolume", None, "pv-1")
            .expect("cluster-scoped object should validate");
    }

    #[test]
    fn rejects_malformed_yaml() {
        let yaml = "apiVersion: v1\nkind: [this is not\n  a valid: mapping\n";
        let err = parse_and_validate_yaml(yaml, "v1", "ConfigMap", Some("default"), "cm")
            .expect_err("malformed YAML must be rejected");
        assert!(err.starts_with("malformed YAML"), "unexpected error: {err}");
    }

    #[test]
    fn rejects_name_change() {
        let yaml = "apiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: renamed\n  namespace: default\ndata: {}\n";
        let err = parse_and_validate_yaml(yaml, "v1", "ConfigMap", Some("default"), "cm")
            .expect_err("name change must be rejected");
        assert!(err.contains("metadata.name"), "unexpected error: {err}");
    }

    #[test]
    fn rejects_kind_change() {
        let yaml = "apiVersion: apps/v1\nkind: StatefulSet\nmetadata:\n  name: web\n  namespace: default\nspec: {}\n";
        let err = parse_and_validate_yaml(yaml, "apps/v1", "Deployment", Some("default"), "web")
            .expect_err("kind change must be rejected");
        assert!(err.contains("kind"), "unexpected error: {err}");
    }

    #[test]
    fn rejects_api_version_change() {
        let yaml = "apiVersion: apps/v1beta1\nkind: Deployment\nmetadata:\n  name: web\n  namespace: default\nspec: {}\n";
        let err = parse_and_validate_yaml(yaml, "apps/v1", "Deployment", Some("default"), "web")
            .expect_err("apiVersion change must be rejected");
        assert!(err.contains("apiVersion"), "unexpected error: {err}");
    }

    #[test]
    fn rejects_namespace_change() {
        let yaml = "apiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: cm\n  namespace: other\ndata: {}\n";
        let err = parse_and_validate_yaml(yaml, "v1", "ConfigMap", Some("default"), "cm")
            .expect_err("namespace change must be rejected");
        assert!(
            err.contains("metadata.namespace"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn rejects_namespace_added_to_cluster_scoped_object() {
        let yaml = "apiVersion: v1\nkind: PersistentVolume\nmetadata:\n  name: pv-1\n  namespace: default\nspec: {}\n";
        let err = parse_and_validate_yaml(yaml, "v1", "PersistentVolume", None, "pv-1")
            .expect_err("adding a namespace to a cluster-scoped object must be rejected");
        assert!(
            err.contains("metadata.namespace"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn split_api_version_handles_core_and_grouped() {
        assert_eq!(split_api_version("v1"), (String::new(), "v1".to_owned()));
        assert_eq!(
            split_api_version("apps/v1"),
            ("apps".to_owned(), "v1".to_owned())
        );
    }
}
