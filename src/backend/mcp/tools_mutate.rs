//! The stdio MCP server's *gated* mutation tools (015-mcp-agent-exposure, T035-T037).
//!
//! Three tools — `scale_workload`, `apply_resource_yaml` and `delete_resource` — let an attached
//! external agent act on the cluster. Constitution Principle I ("Operator-First, Explicit
//! Mutation") is enforced here by three things that cannot be skipped:
//!
//! 1. **The schema.** Every tool's input struct carries a mandatory, non-`Option`
//!    `expected_resource_version: String`. It is a required JSON-schema field with no default, so
//!    a call that omits it fails deserialization and never reaches a handler at all
//!    (`mutation_tool_schemas_require_expected_resource_version` asserts the schema; the
//!    handler-level tests assert the rejection).
//! 2. **RBAC, first.** Each handler calls [`super::tools_read::FathomMcpServer::authorize`] with
//!    a *mutating* verb (`update`/`delete`) before anything else touches the cluster, so a call
//!    scoped outside Fathom's own cluster identity is refused before the version guard runs and
//!    before any write is attempted.
//! 3. **The version guard, re-validated server side.** [`guard_resource_version`] re-reads the
//!    live target and compares its `metadata.resourceVersion` (and `uid`, when the caller
//!    supplied one) against what the agent presented. Client-side approval is never trusted: a
//!    stale or empty value is an [`McpDenialCategory::MutationGuardRejected`] refusal and
//!    nothing is written.
//!
//! Only after all three pass does a handler call an *existing* backend mutation function —
//! [`crate::backend::apply::apply_resource`] for apply (the exact function the UI's own
//! `ApplyResourceYaml` command calls), and `deployments`/`statefulsets`/`pods`'s
//! `*_with_client` functions for scale and delete. Those functions also own the event emission,
//! so an MCP-driven mutation emits the same `BackendEvent` a UI-driven one does and the
//! operator's own Fathom window reflects it live (T037). There is no second mutation code path.

use kube::ResourceExt;
use kube::api::{Api, DynamicObject};
use kube::core::{ApiResource, GroupVersionKind};
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock};
use rmcp::{ErrorData as McpError, tool, tool_router};
use serde::{Deserialize, Serialize};

use super::rbac_guard::McpToolDenial;
use super::tools_read::{FathomMcpServer, denied, dynamic_api, plural_for_kind};

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub(super) struct ScaleWorkloadParams {
    /// Workload kind to scale: `Deployment` or `StatefulSet`.
    pub kind: String,
    pub namespace: String,
    pub name: String,
    /// Desired replica count.
    pub replicas: i32,
    /// The target's current `metadata.resourceVersion`, from a prior read. Required.
    pub expected_resource_version: String,
    /// Optionally also pin `metadata.uid`, so a delete-and-recreate between the read and this
    /// call is refused rather than silently scaling the replacement object.
    #[serde(default)]
    pub expected_uid: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub(super) struct ApplyResourceYamlParams {
    /// The complete object manifest to apply. Its `apiVersion`, `kind` and `metadata` identify
    /// the target; this tool replaces an existing object and never creates a new one.
    pub yaml: String,
    /// The target's current `metadata.resourceVersion`, from a prior read. Required.
    pub expected_resource_version: String,
    #[serde(default)]
    pub expected_uid: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub(super) struct DeleteResourceParams {
    /// Kind to delete: `Pod`, `Deployment` or `StatefulSet`.
    pub kind: String,
    pub namespace: String,
    pub name: String,
    /// The target's current `metadata.resourceVersion`, from a prior read. Required.
    pub expected_resource_version: String,
    #[serde(default)]
    pub expected_uid: Option<String>,
}

/// The `apiVersion` for a kind this module can scale or delete. Deliberately a closed list:
/// these are exactly the kinds with an existing, event-emitting backend mutation function to
/// reuse, and an unknown kind is refused rather than handled by new bespoke write code.
fn workload_api_version(kind: &str) -> Option<&'static str> {
    match kind {
        "Pod" => Some("v1"),
        "Deployment" | "StatefulSet" => Some("apps/v1"),
        _ => None,
    }
}

fn unsupported_kind(tool: &str, kind: &str, supported: &str) -> McpToolDenial {
    McpToolDenial::mutation_guard(
        tool,
        format!(
            "'{kind}' is not a kind this tool can change (supported: {supported}). Nothing was \
             written to the cluster."
        ),
    )
}

/// An error result for a failure that is not a guard refusal (the write itself failed, the
/// event bus is missing, …). Distinct from [`denied`] so refusals stay recognisable.
fn failed(tool: &str, message: String) -> CallToolResult {
    CallToolResult::error(vec![ContentBlock::text(format!(
        "{tool} failed: {message}"
    ))])
}

/// Re-read the live target and check it against what the agent presented (T036 step 2).
///
/// This is the whole point of the phase: the `expected_resource_version` an MCP client sends is
/// *evidence it read the object*, not authorization. It is checked here, server side, against
/// the cluster's current state, immediately before the write. Any mismatch — including an empty
/// value, an object that has since been deleted, or a `uid` change from a delete-and-recreate —
/// refuses the call, and no write function is ever reached.
async fn guard_resource_version(
    api: &Api<DynamicObject>,
    tool: &str,
    kind: &str,
    namespace: &str,
    name: &str,
    expected_resource_version: &str,
    expected_uid: Option<&str>,
) -> Result<(), McpToolDenial> {
    if expected_resource_version.trim().is_empty() {
        return Err(McpToolDenial::mutation_guard(
            tool,
            format!(
                "no expected_resource_version was supplied for {kind} {namespace}/{name}. Read \
                 the object first and pass its current metadata.resourceVersion. Nothing was \
                 written to the cluster."
            ),
        ));
    }

    let live = api.get(name).await.map_err(|e| {
        McpToolDenial::mutation_guard(
            tool,
            format!(
                "could not re-read {kind} {namespace}/{name} to verify it is unchanged ({e}). \
                 Nothing was written to the cluster."
            ),
        )
    })?;

    let live_version = live.resource_version().unwrap_or_default();
    if live_version != expected_resource_version {
        return Err(McpToolDenial::mutation_guard(
            tool,
            format!(
                "{kind} {namespace}/{name} has changed since it was read (expected \
                 resourceVersion {expected_resource_version}, live is {live_version}). Refresh \
                 and retry. Nothing was written to the cluster."
            ),
        ));
    }

    if let Some(expected_uid) = expected_uid {
        let live_uid = live.uid().unwrap_or_default();
        if live_uid != expected_uid {
            return Err(McpToolDenial::mutation_guard(
                tool,
                format!(
                    "{kind} {namespace}/{name} is a different object than the one that was read \
                     (expected uid {expected_uid}, live is {live_uid}). Refresh and retry. \
                     Nothing was written to the cluster."
                ),
            ));
        }
    }

    Ok(())
}

/// A `DynamicObject` API for a known kind, built without a discovery round-trip: the kinds this
/// module scales and deletes are a closed list, so their plural names are known.
fn known_kind_api(
    client: &kube::Client,
    api_version: &str,
    kind: &str,
    namespace: &str,
) -> Api<DynamicObject> {
    let (group, version) = match api_version.split_once('/') {
        Some((group, version)) => (group, version),
        None => ("", api_version),
    };
    let gvk = GroupVersionKind::gvk(group, version, kind);
    let api_resource = ApiResource::from_gvk_with_plural(&gvk, &plural_for_kind(kind));
    Api::namespaced_with(client.clone(), namespace, &api_resource)
}

#[tool_router(router = mutate_tool_router, vis = "pub(super)")]
impl FathomMcpServer {
    #[tool(
        description = "Scale a Deployment or StatefulSet to a given replica count. This action \
                       modifies the cluster. Confirm with the operator before calling. Requires \
                       expected_resource_version from a prior read; Fathom re-validates it \
                       against the live object and refuses if it is stale.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = true
        )
    )]
    async fn scale_workload(
        &self,
        Parameters(params): Parameters<ScaleWorkloadParams>,
    ) -> Result<CallToolResult, McpError> {
        const TOOL: &str = "scale_workload";
        let Some(api_version) = workload_api_version(&params.kind)
            .filter(|_| params.kind == "Deployment" || params.kind == "StatefulSet")
        else {
            return Ok(denied(&unsupported_kind(
                TOOL,
                &params.kind,
                "Deployment, StatefulSet",
            )));
        };

        // 1. RBAC, before the version guard and before any write.
        let client = match self
            .authorize(
                TOOL,
                Some(params.namespace.as_str()),
                "update",
                &plural_for_kind(&params.kind),
            )
            .await
        {
            Ok(client) => client,
            Err(denial) => return Ok(denied(&denial)),
        };

        // 2. Re-validate the caller's evidence against the live object.
        let api = known_kind_api(&client, api_version, &params.kind, &params.namespace);
        if let Err(denial) = guard_resource_version(
            &api,
            TOOL,
            &params.kind,
            &params.namespace,
            &params.name,
            &params.expected_resource_version,
            params.expected_uid.as_deref(),
        )
        .await
        {
            return Ok(denied(&denial));
        }

        let Some((seq, sender)) = self.cluster.events() else {
            return Ok(failed(TOOL, NO_EVENT_BUS.to_owned()));
        };
        let context = self.cluster.context();

        // 3. The existing backend scale function — the same one `ScaleDeployment` /
        //    `ScaleStatefulSet` call, event emission included.
        let outcome = match params.kind.as_str() {
            "Deployment" => {
                crate::backend::deployments::scale_deployment_with_client(
                    &client,
                    seq.as_ref(),
                    &sender,
                    &params.namespace,
                    &params.name,
                    params.replicas,
                )
                .await
            }
            _ => {
                crate::backend::statefulsets::scale_statefulset_with_client(
                    &client,
                    seq.as_ref(),
                    &sender,
                    context.as_deref(),
                    &params.namespace,
                    &params.name,
                    params.replicas,
                )
                .await
            }
        };

        Ok(match outcome {
            Ok(()) => CallToolResult::success(vec![ContentBlock::text(format!(
                "Scaled {} {}/{} to {} replicas.",
                params.kind, params.namespace, params.name, params.replicas
            ))]),
            Err(e) => failed(TOOL, format!("{e:#}")),
        })
    }

    #[tool(
        description = "Replace one existing Kubernetes object with the supplied YAML manifest. \
                       This action modifies the cluster. Confirm with the operator before \
                       calling. Requires expected_resource_version from a prior read; Fathom \
                       re-validates it against the live object and refuses if it is stale.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = true
        )
    )]
    async fn apply_resource_yaml(
        &self,
        Parameters(params): Parameters<ApplyResourceYamlParams>,
    ) -> Result<CallToolResult, McpError> {
        const TOOL: &str = "apply_resource_yaml";

        let identity = match ManifestIdentity::parse(&params.yaml) {
            Ok(identity) => identity,
            Err(message) => return Ok(denied(&McpToolDenial::mutation_guard(TOOL, message))),
        };

        // 1. RBAC, before the version guard and before any write.
        let client = match self
            .authorize(
                TOOL,
                identity.namespace.as_deref(),
                "update",
                &plural_for_kind(&identity.kind),
            )
            .await
        {
            Ok(client) => client,
            Err(denial) => return Ok(denied(&denial)),
        };

        // 2. Re-validate against the live object. Discovery-based, matching `apply_resource`'s
        //    own dispatch, so an arbitrary (including CRD) kind resolves the same way here as
        //    it will during the write.
        let api = match dynamic_api(
            &client,
            &identity.api_version,
            &identity.kind,
            identity.namespace.as_deref(),
        )
        .await
        {
            Ok(api) => api,
            Err(e) => return Ok(failed(TOOL, e.to_string())),
        };
        if let Err(denial) = guard_resource_version(
            &api,
            TOOL,
            &identity.kind,
            identity.namespace.as_deref().unwrap_or("<cluster>"),
            &identity.name,
            &params.expected_resource_version,
            params.expected_uid.as_deref(),
        )
        .await
        {
            return Ok(denied(&denial));
        }

        // The UI's own apply path validates the edited YAML's identity fields against the
        // session it was opened from; here the identity *is* the manifest's, so this re-check
        // only catches a self-inconsistent document, and costs nothing.
        let object = match crate::backend::apply::parse_and_validate_yaml(
            &params.yaml,
            &identity.api_version,
            &identity.kind,
            identity.namespace.as_deref(),
            &identity.name,
        ) {
            Ok(object) => object,
            Err(message) => return Ok(denied(&McpToolDenial::mutation_guard(TOOL, message))),
        };

        let Some((seq, sender)) = self.cluster.events() else {
            return Ok(failed(TOOL, NO_EVENT_BUS.to_owned()));
        };

        // 3. The exact function `ApplyResourceYaml`'s own handler calls, with the same
        //    optimistic-concurrency argument.
        match crate::backend::apply::apply_resource(
            &client,
            &identity.api_version,
            &identity.kind,
            identity.namespace.as_deref(),
            &identity.name,
            object,
            &params.expected_resource_version,
        )
        .await
        {
            Ok(resource_version) => {
                crate::backend::apply::emit_resource_applied(
                    seq.as_ref(),
                    &sender,
                    resource_version.clone(),
                );
                Ok(CallToolResult::success(vec![ContentBlock::text(format!(
                    "Applied {} {}. New resourceVersion: {resource_version}.",
                    identity.kind, identity.name
                ))]))
            }
            Err(e) => Ok(failed(TOOL, e.to_string())),
        }
    }

    #[tool(
        description = "Delete one Pod, Deployment or StatefulSet. This action modifies the \
                       cluster. Confirm with the operator before calling. Requires \
                       expected_resource_version from a prior read; Fathom re-validates it \
                       against the live object and refuses if it is stale.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false
        )
    )]
    async fn delete_resource(
        &self,
        Parameters(params): Parameters<DeleteResourceParams>,
    ) -> Result<CallToolResult, McpError> {
        const TOOL: &str = "delete_resource";
        let Some(api_version) = workload_api_version(&params.kind) else {
            return Ok(denied(&unsupported_kind(
                TOOL,
                &params.kind,
                "Pod, Deployment, StatefulSet",
            )));
        };

        // 1. RBAC, before the version guard and before any write.
        let client = match self
            .authorize(
                TOOL,
                Some(params.namespace.as_str()),
                "delete",
                &plural_for_kind(&params.kind),
            )
            .await
        {
            Ok(client) => client,
            Err(denial) => return Ok(denied(&denial)),
        };

        // 2. Re-validate the caller's evidence against the live object.
        let api = known_kind_api(&client, api_version, &params.kind, &params.namespace);
        if let Err(denial) = guard_resource_version(
            &api,
            TOOL,
            &params.kind,
            &params.namespace,
            &params.name,
            &params.expected_resource_version,
            params.expected_uid.as_deref(),
        )
        .await
        {
            return Ok(denied(&denial));
        }

        let Some((seq, sender)) = self.cluster.events() else {
            return Ok(failed(TOOL, NO_EVENT_BUS.to_owned()));
        };
        let context = self.cluster.context();

        // 3. The existing backend delete function for this kind — the same one `DeletePod` /
        //    `DeleteDeployment` / `DeleteStatefulSet` call, event emission included.
        let outcome = match params.kind.as_str() {
            "Pod" => {
                crate::backend::pods::delete_pod_with_client(
                    &client,
                    seq.as_ref(),
                    &sender,
                    context.as_deref(),
                    &params.namespace,
                    &params.name,
                )
                .await
            }
            "Deployment" => {
                crate::backend::deployments::delete_deployment_with_client(
                    &client,
                    seq.as_ref(),
                    &sender,
                    &params.namespace,
                    &params.name,
                )
                .await
            }
            _ => {
                crate::backend::statefulsets::delete_statefulset_with_client(
                    &client,
                    seq.as_ref(),
                    &sender,
                    context.as_deref(),
                    &params.namespace,
                    &params.name,
                )
                .await
            }
        };

        Ok(match outcome {
            Ok(()) => CallToolResult::success(vec![ContentBlock::text(format!(
                "Deleted {} {}/{}.",
                params.kind, params.namespace, params.name
            ))]),
            Err(e) => failed(TOOL, format!("{e:#}")),
        })
    }
}

/// Only reachable when the server task was started without the backend publishing its event bus
/// — impossible through `SetMcpServerEnabled`'s reconcile path, which publishes both together.
/// Checked *before* the write, so an un-reportable mutation is refused rather than performed
/// silently (Principle I: the UI must reflect every change).
const NO_EVENT_BUS: &str = "Fathom's event bus is not attached to the MCP server, so this \
                            mutation could not be reported to the operator's own UI. Nothing \
                            was written to the cluster.";

/// The `apiVersion`/`kind`/`metadata` a manifest names, i.e. which object it targets.
struct ManifestIdentity {
    api_version: String,
    kind: String,
    namespace: Option<String>,
    name: String,
}

impl ManifestIdentity {
    fn parse(yaml: &str) -> Result<Self, String> {
        let value: serde_json::Value = serde_yaml::from_str(yaml)
            .map_err(|e| format!("malformed YAML: could not parse the manifest: {e}"))?;
        let api_version = value
            .get("apiVersion")
            .and_then(|v| v.as_str())
            .ok_or("the manifest has no apiVersion, so its target is ambiguous")?
            .to_owned();
        let kind = value
            .get("kind")
            .and_then(|v| v.as_str())
            .ok_or("the manifest has no kind, so its target is ambiguous")?
            .to_owned();
        let metadata = value.get("metadata");
        let name = metadata
            .and_then(|m| m.get("name"))
            .and_then(|v| v.as_str())
            .ok_or("the manifest has no metadata.name, so its target is ambiguous")?
            .to_owned();
        let namespace = metadata
            .and_then(|m| m.get("namespace"))
            .and_then(|v| v.as_str())
            .map(str::to_owned);
        Ok(Self {
            api_version,
            kind,
            namespace,
            name,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::mcp::rbac_guard::McpDenialCategory;
    use crate::protocol::{BackendEvent, SystemEvent};
    use k8s_openapi::api::rbac::v1::{
        ClusterRole, ClusterRoleBinding, PolicyRule, RoleRef, Subject,
    };
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
    use std::sync::Arc;
    use std::sync::atomic::AtomicU64;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use crate::backend::mcp::McpClusterHandle;

    const LIVE_RESOURCE_VERSION: &str = "4021";

    /// Same fixture convention as `tools_read`'s tests (and `apply.rs`'s own): a real
    /// `kube::Client` pointed at a `wiremock` server, so every request the code would make is
    /// observable and nothing needs a live cluster.
    fn client_for(url: &str) -> kube::Client {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let config = kube::Config::new(url.parse().expect("mock server url"));
        kube::Client::try_from(config).expect("client from mock config")
    }

    /// Grants the reviewed identity full read *and* write access, so the RBAC gate is never
    /// what refuses in these tests — the resource-version guard is.
    fn writer_rbac() -> (ClusterRole, ClusterRoleBinding) {
        (
            ClusterRole {
                metadata: ObjectMeta {
                    name: Some("fathom-writer".to_owned()),
                    ..Default::default()
                },
                rules: Some(vec![PolicyRule {
                    api_groups: Some(vec![String::new(), "apps".to_owned()]),
                    resources: Some(vec!["*".to_owned()]),
                    verbs: vec![
                        "get".to_owned(),
                        "list".to_owned(),
                        "update".to_owned(),
                        "patch".to_owned(),
                        "delete".to_owned(),
                    ],
                    ..Default::default()
                }]),
                aggregation_rule: None,
            },
            ClusterRoleBinding {
                metadata: ObjectMeta {
                    name: Some("fathom-writer".to_owned()),
                    ..Default::default()
                },
                role_ref: RoleRef {
                    api_group: "rbac.authorization.k8s.io".to_owned(),
                    kind: "ClusterRole".to_owned(),
                    name: "fathom-writer".to_owned(),
                },
                subjects: Some(vec![Subject {
                    kind: "User".to_owned(),
                    name: "fathom-tester".to_owned(),
                    namespace: None,
                    api_group: None,
                }]),
            },
        )
    }

    /// An API server that answers "who am I" + the RBAC listing + a GET of one live Deployment
    /// at [`LIVE_RESOURCE_VERSION`]. **No write route is mounted**: any PATCH/PUT/DELETE the
    /// code attempted would hit wiremock's "no matching mock" path and, more importantly, show
    /// up in [`write_requests`].
    async fn mock_apiserver() -> MockServer {
        let server = MockServer::start().await;
        let (cluster_role, binding) = writer_rbac();

        Mock::given(method("POST"))
            .and(path("/apis/authentication.k8s.io/v1/selfsubjectreviews"))
            .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
                "apiVersion": "authentication.k8s.io/v1",
                "kind": "SelfSubjectReview",
                "status": { "userInfo": { "username": "fathom-tester" } }
            })))
            .mount(&server)
            .await;

        for (p, body) in [
            (
                "/apis/rbac.authorization.k8s.io/v1/roles",
                serde_json::json!({ "apiVersion": "v1", "kind": "RoleList", "items": [] }),
            ),
            (
                "/apis/rbac.authorization.k8s.io/v1/clusterroles",
                serde_json::json!({
                    "apiVersion": "v1", "kind": "ClusterRoleList", "items": [cluster_role]
                }),
            ),
            (
                "/apis/rbac.authorization.k8s.io/v1/rolebindings",
                serde_json::json!({ "apiVersion": "v1", "kind": "RoleBindingList", "items": [] }),
            ),
            (
                "/apis/rbac.authorization.k8s.io/v1/clusterrolebindings",
                serde_json::json!({
                    "apiVersion": "v1", "kind": "ClusterRoleBindingList", "items": [binding]
                }),
            ),
        ] {
            Mock::given(method("GET"))
                .and(path(p))
                .respond_with(ResponseTemplate::new(200).set_body_json(body))
                .mount(&server)
                .await;
        }

        Mock::given(method("GET"))
            .and(path("/apis/apps/v1/namespaces/payments/deployments/api"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "apiVersion": "apps/v1",
                "kind": "Deployment",
                "metadata": {
                    "name": "api",
                    "namespace": "payments",
                    "resourceVersion": LIVE_RESOURCE_VERSION,
                    "uid": "0f4f-live"
                },
                "spec": { "replicas": 3 }
            })))
            .mount(&server)
            .await;

        server
    }

    /// Every request the code made that could have changed the cluster.
    ///
    /// `POST /selfsubjectreviews` is the "who am I" subresource the RBAC guard issues; it
    /// persists nothing and is excluded by path. Everything else non-GET is a write.
    async fn write_requests(server: &MockServer) -> Vec<String> {
        server
            .received_requests()
            .await
            .unwrap_or_default()
            .iter()
            .filter(|r| {
                r.method != wiremock::http::Method::GET
                    && !r.url.path().ends_with("/selfsubjectreviews")
            })
            .map(|r| format!("{} {}", r.method, r.url.path()))
            .collect()
    }

    fn server_for(
        client: kube::Client,
    ) -> (
        FathomMcpServer,
        crossbeam_channel::Receiver<crate::protocol::BackendEventEnvelope>,
    ) {
        let (tx, rx) = crossbeam_channel::unbounded();
        let handle = McpClusterHandle::default();
        handle.publish(Some(client), Some("mock".to_owned()));
        handle.publish_events(Arc::new(AtomicU64::new(0)), tx);
        (FathomMcpServer::new(handle), rx)
    }

    fn text_of(result: &CallToolResult) -> String {
        result
            .content
            .first()
            .and_then(|c| c.as_text())
            .map(|t| t.text.clone())
            .unwrap_or_default()
    }

    /// T032 / spec.md US2 acceptance 1-2: a mutation tool cannot run without presenting the
    /// target's *current* resource version, and a rejected call writes nothing at all.
    ///
    /// Four ways of trying to skip the guard are exercised:
    ///   (a) omitting `expected_resource_version` entirely — rejected at deserialization,
    ///       because the field is a mandatory `String`, not an `Option`, so no handler runs;
    ///   (b) sending an empty value;
    ///   (c) sending a stale value that does not match the live fixture object;
    ///   (d) sending a matching version but a `uid` from a since-recreated object.
    /// In (b)-(d) the assertion that matters is the last one: `write_requests` — every non-GET
    /// request the code actually issued — is exactly empty.
    #[tokio::test]
    async fn mcp_mutation_tools_require_explicit_approval() {
        // (a) Structural: the schema/deserializer rejects an omitted resource version, so a
        // handler is never entered. This is what makes "missing" untestable at the handler
        // level — it cannot be constructed.
        let omitted = serde_json::json!({
            "kind": "Deployment",
            "namespace": "payments",
            "name": "api",
            "replicas": 1
        });
        let err = serde_json::from_value::<ScaleWorkloadParams>(omitted)
            .expect_err("expected_resource_version is mandatory, not optional");
        assert!(
            err.to_string().contains("expected_resource_version"),
            "the deserializer must name the missing guard field: {err}"
        );
        assert!(
            serde_json::from_value::<DeleteResourceParams>(serde_json::json!({
                "kind": "Deployment", "namespace": "payments", "name": "api"
            }))
            .is_err()
        );
        assert!(
            serde_json::from_value::<ApplyResourceYamlParams>(serde_json::json!({
                "yaml": "apiVersion: v1\nkind: ConfigMap\n"
            }))
            .is_err()
        );

        let server = mock_apiserver().await;
        let (tools, events) = server_for(client_for(&server.uri()));

        // (b) empty, (c) stale.
        for version in ["", "1"] {
            let result = tools
                .scale_workload(Parameters(ScaleWorkloadParams {
                    kind: "Deployment".to_owned(),
                    namespace: "payments".to_owned(),
                    name: "api".to_owned(),
                    replicas: 10,
                    expected_resource_version: version.to_owned(),
                    expected_uid: None,
                }))
                .await
                .expect("scale_workload should answer");
            assert_eq!(
                result.is_error,
                Some(true),
                "resource version {version:?} must be refused"
            );
            let text = text_of(&result);
            assert!(text.starts_with("scale_workload refused:"), "{text}");
            assert!(
                text.contains("Nothing was written to the cluster"),
                "the refusal must state that nothing was written: {text}"
            );

            let result = tools
                .delete_resource(Parameters(DeleteResourceParams {
                    kind: "Deployment".to_owned(),
                    namespace: "payments".to_owned(),
                    name: "api".to_owned(),
                    expected_resource_version: version.to_owned(),
                    expected_uid: None,
                }))
                .await
                .expect("delete_resource should answer");
            assert_eq!(result.is_error, Some(true));
            let text = text_of(&result);
            assert!(text.starts_with("delete_resource refused:"), "{text}");
            assert!(
                text.contains("Nothing was written to the cluster"),
                "{text}"
            );
        }

        // (d) right version, wrong object identity.
        let result = tools
            .delete_resource(Parameters(DeleteResourceParams {
                kind: "Deployment".to_owned(),
                namespace: "payments".to_owned(),
                name: "api".to_owned(),
                expected_resource_version: LIVE_RESOURCE_VERSION.to_owned(),
                expected_uid: Some("0f4f-recreated".to_owned()),
            }))
            .await
            .expect("delete_resource should answer");
        assert_eq!(result.is_error, Some(true));
        assert!(
            text_of(&result).contains("different object"),
            "{}",
            text_of(&result)
        );

        // The assertion this whole test exists for: across every rejected call above, the
        // fixture API server recorded ZERO writes.
        let writes = write_requests(&server).await;
        assert!(
            writes.is_empty(),
            "a rejected mutation must not write to the cluster, but these requests were sent: \
             {writes:?}"
        );
        assert!(
            events.try_iter().next().is_none(),
            "a rejected mutation must not announce anything to the UI either"
        );
    }

    /// The positive control for the test above, and T037: when the presented version *does*
    /// match the live object, the tool performs the write through the existing backend
    /// function and that function emits the same `Notification` the UI-driven
    /// `ScaleDeployment` command emits.
    #[tokio::test]
    async fn a_current_resource_version_lets_the_scale_through_and_reports_it_to_the_ui() {
        let server = mock_apiserver().await;
        Mock::given(method("PATCH"))
            .and(path("/apis/apps/v1/namespaces/payments/deployments/api"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "apiVersion": "apps/v1",
                "kind": "Deployment",
                "metadata": { "name": "api", "namespace": "payments", "resourceVersion": "4022" },
                "spec": { "replicas": 10 }
            })))
            .mount(&server)
            .await;

        let (tools, events) = server_for(client_for(&server.uri()));
        let result = tools
            .scale_workload(Parameters(ScaleWorkloadParams {
                kind: "Deployment".to_owned(),
                namespace: "payments".to_owned(),
                name: "api".to_owned(),
                replicas: 10,
                expected_resource_version: LIVE_RESOURCE_VERSION.to_owned(),
                expected_uid: Some("0f4f-live".to_owned()),
            }))
            .await
            .expect("scale_workload should answer");
        assert_eq!(result.is_error, Some(false), "{}", text_of(&result));

        let writes = write_requests(&server).await;
        assert_eq!(
            writes,
            vec!["PATCH /apis/apps/v1/namespaces/payments/deployments/api".to_owned()],
            "exactly one write, issued by the existing scale_deployment_with_client function"
        );

        let notified = events.try_iter().any(|e| {
            matches!(
                &e.event,
                BackendEvent::System(SystemEvent::Notification { message, .. })
                    if message == "Scaled deployment payments/api to 10"
            )
        });
        assert!(
            notified,
            "the operator's own UI must be told, through the same emission the UI-driven scale \
             uses"
        );
    }

    /// The RBAC gate runs *before* the version guard: an identity with no grants is refused
    /// without the target ever being re-read, and of course without any write.
    #[tokio::test]
    async fn rbac_refusal_happens_before_the_version_guard_and_writes_nothing() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/apis/authentication.k8s.io/v1/selfsubjectreviews"))
            .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
                "apiVersion": "authentication.k8s.io/v1",
                "kind": "SelfSubjectReview",
                "status": { "userInfo": { "username": "nobody" } }
            })))
            .mount(&server)
            .await;
        for (p, kind) in [
            ("/apis/rbac.authorization.k8s.io/v1/roles", "RoleList"),
            (
                "/apis/rbac.authorization.k8s.io/v1/clusterroles",
                "ClusterRoleList",
            ),
            (
                "/apis/rbac.authorization.k8s.io/v1/rolebindings",
                "RoleBindingList",
            ),
            (
                "/apis/rbac.authorization.k8s.io/v1/clusterrolebindings",
                "ClusterRoleBindingList",
            ),
        ] {
            Mock::given(method("GET"))
                .and(path(p))
                .respond_with(ResponseTemplate::new(200).set_body_json(
                    serde_json::json!({ "apiVersion": "v1", "kind": kind, "items": [] }),
                ))
                .mount(&server)
                .await;
        }

        let (tools, _events) = server_for(client_for(&server.uri()));
        let result = tools
            .delete_resource(Parameters(DeleteResourceParams {
                kind: "Pod".to_owned(),
                namespace: "payments".to_owned(),
                name: "api-7d9".to_owned(),
                expected_resource_version: "whatever".to_owned(),
                expected_uid: None,
            }))
            .await
            .expect("delete_resource should answer");
        assert_eq!(result.is_error, Some(true));
        let text = text_of(&result);
        assert!(text.contains("cannot 'delete' pods"), "{text}");
        assert!(text.contains("Nothing was read from the cluster"), "{text}");

        // No GET of the pod was ever mounted, and none was issued: the guard refused first.
        let reads: Vec<String> = server
            .received_requests()
            .await
            .unwrap_or_default()
            .iter()
            .map(|r| r.url.path().to_owned())
            .filter(|p| p.contains("/pods"))
            .collect();
        assert!(
            reads.is_empty(),
            "RBAC must refuse before the target is re-read: {reads:?}"
        );
        assert!(write_requests(&server).await.is_empty());
    }

    /// T034: the contract that makes "read, then approve, then mutate" mechanical rather than
    /// conventional — every mutation tool's *published* JSON schema lists
    /// `expected_resource_version` as required, with no default, so a compliant MCP client
    /// cannot construct a call without having read the object first.
    #[test]
    fn mutation_tool_schemas_require_expected_resource_version() {
        for tool in [
            FathomMcpServer::scale_workload_tool_attr(),
            FathomMcpServer::apply_resource_yaml_tool_attr(),
            FathomMcpServer::delete_resource_tool_attr(),
        ] {
            let schema = serde_json::Value::Object((*tool.input_schema).clone());
            let required: Vec<&str> = schema
                .get("required")
                .and_then(|r| r.as_array())
                .map(|r| r.iter().filter_map(|v| v.as_str()).collect())
                .unwrap_or_default();
            assert!(
                required.contains(&"expected_resource_version"),
                "{}'s schema must require expected_resource_version, got required={required:?}",
                tool.name
            );

            let property = schema
                .get("properties")
                .and_then(|p| p.get("expected_resource_version"))
                .unwrap_or_else(|| {
                    panic!("{} has no expected_resource_version property", tool.name)
                });
            assert!(
                property.get("default").is_none(),
                "{}'s expected_resource_version must have no default: {property}",
                tool.name
            );
            assert_eq!(
                property.get("type").and_then(|t| t.as_str()),
                Some("string")
            );

            let description = tool.description.as_deref().unwrap_or_default();
            assert!(
                description.contains("modifies the cluster")
                    && description.contains("Confirm with the operator"),
                "{}'s description must warn the client to confirm: {description}",
                tool.name
            );
            let annotations = tool
                .annotations
                .as_ref()
                .unwrap_or_else(|| panic!("{} must be annotated", tool.name));
            assert_eq!(annotations.read_only_hint, Some(false));
            assert_eq!(annotations.destructive_hint, Some(true));
        }
    }

    #[test]
    fn a_kind_with_no_existing_backend_mutation_function_is_refused() {
        let denial = unsupported_kind("scale_workload", "CronJob", "Deployment, StatefulSet");
        assert_eq!(denial.category, McpDenialCategory::MutationGuardRejected);
        assert!(denial.message().contains("Nothing was written"));
        assert!(workload_api_version("CronJob").is_none());
        assert_eq!(workload_api_version("Pod"), Some("v1"));
        assert_eq!(workload_api_version("Deployment"), Some("apps/v1"));
    }

    #[test]
    fn a_manifest_without_identity_fields_is_refused_before_anything_else() {
        assert!(ManifestIdentity::parse("kind: ConfigMap\nmetadata:\n  name: x\n").is_err());
        assert!(ManifestIdentity::parse("apiVersion: v1\nmetadata:\n  name: x\n").is_err());
        assert!(ManifestIdentity::parse("apiVersion: v1\nkind: ConfigMap\n").is_err());
        let identity = ManifestIdentity::parse(
            "apiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: cm\n  namespace: payments\n",
        )
        .expect("a complete manifest identifies its target");
        assert_eq!(identity.api_version, "v1");
        assert_eq!(identity.kind, "ConfigMap");
        assert_eq!(identity.namespace.as_deref(), Some("payments"));
        assert_eq!(identity.name, "cm");
    }
}
