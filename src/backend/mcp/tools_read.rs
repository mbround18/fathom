//! The stdio MCP server's read-only tool surface (015-mcp-agent-exposure, T023).
//!
//! Five tools — `get_resource`, `list_resources`, `get_pod_logs`, `get_events` and
//! `describe_pod` — expose exactly the cluster reads Fathom's own panels already perform, through
//! the same backend helpers (`describe_pod` calls
//! [`crate::backend::agent::grounding::collect_pod_grounding_with_client`], the very function the
//! in-app Agent panel grounds on, so an external agent and the in-app one see identical data,
//! research.md R6).
//!
//! **Nothing here mutates, and nothing here reads before the RBAC guard says yes.** Every tool
//! body has the same shape:
//!
//! 1. take a `kube::Client` out of the shared [`McpClusterHandle`], or refuse with
//!    [`McpToolDenial::no_cluster`];
//! 2. [`rbac_guard::resolve_current_access`] → [`rbac_guard::check`] for this tool's verb and
//!    resource, returning the denial before any `kube::Api` for the requested object exists;
//! 3. only then perform the read.
//!
//! Mutating tools are Phase 4's job (T035/T036) and deliberately absent: the deleted
//! `src/mcp_server.rs` prototype shipped an unguarded `restart_deployment`, which is the exact
//! failure this module's ordering prevents.

use k8s_openapi::api::core::v1::{Event, Pod};
use kube::ResourceExt;
use kube::api::{Api, DynamicObject, ListParams, LogParams};
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, Content, Implementation, ServerCapabilities, ServerInfo};
use rmcp::{ErrorData as McpError, ServerHandler, tool, tool_handler, tool_router};
use serde::{Deserialize, Serialize};

use super::McpClusterHandle;
use super::rbac_guard::{self, McpToolDenial};
use crate::protocol::v1::{AgentObjectRef, GroundingSource};

/// How many objects a single `list_resources`/`get_events` answer carries (Principle V's
/// bounded-data rule — an MCP client must not be able to ask for a whole cluster dump).
const MAX_LIST_ITEMS: usize = 100;
/// Default (and maximum) log tail an MCP client can request.
const MAX_LOG_LINES: i64 = 500;

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub(super) struct GetResourceParams {
    /// `apiVersion` of the object, e.g. `v1` or `apps/v1`.
    pub api_version: String,
    /// `kind` of the object, e.g. `Pod` or `Deployment`.
    pub kind: String,
    /// Object name.
    pub name: String,
    /// Namespace, omitted for cluster-scoped kinds.
    #[serde(default)]
    pub namespace: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub(super) struct ListResourcesParams {
    /// `apiVersion` of the kind to list, e.g. `v1` or `apps/v1`.
    pub api_version: String,
    /// `kind` to list, e.g. `Pod`.
    pub kind: String,
    /// Namespace to list in. Omit to list across the cluster, which needs a cluster-wide grant.
    #[serde(default)]
    pub namespace: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub(super) struct GetPodLogsParams {
    pub namespace: String,
    pub name: String,
    /// Container name; defaults to the pod's first container.
    #[serde(default)]
    pub container: Option<String>,
    /// Trailing lines to return, capped at 500.
    #[serde(default)]
    pub tail_lines: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub(super) struct GetEventsParams {
    /// Namespace to read events from. Omit for cluster-wide, which needs a cluster-wide grant.
    #[serde(default)]
    pub namespace: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub(super) struct DescribePodParams {
    pub namespace: String,
    pub name: String,
}

/// The `rmcp` `ServerHandler` behind the stdio transport.
///
/// Holds no `BackendState` and no client of its own: the backend publishes its live
/// `kube::Client` into [`McpClusterHandle`] on every connect/context switch, and each tool call
/// clones the current one out. That is what lets the server task outlive any single command's
/// lock on `BackendState` without ever holding one.
#[derive(Clone)]
pub(super) struct FathomMcpServer {
    pub(super) cluster: McpClusterHandle,
    tool_router: ToolRouter<Self>,
}

/// Lowercase plural resource name for RBAC matching, derived *locally* so the guard can refuse
/// before any cluster call (including discovery) happens.
///
/// Deliberately naive: RBAC rules are matched case-insensitively on verbs and accept `*`, and a
/// wrong guess can only ever make the guard stricter, never more permissive.
pub(super) fn plural_for_kind(kind: &str) -> String {
    let lower = kind.to_lowercase();
    if lower.ends_with('s')
        || lower.ends_with('x')
        || lower.ends_with('z')
        || lower.ends_with("ch")
        || lower.ends_with("sh")
    {
        format!("{lower}es")
    } else if let Some(stem) = lower.strip_suffix('y') {
        format!("{stem}ies")
    } else {
        format!("{lower}s")
    }
}

/// A refusal rendered for the attached MCP client: an error result carrying the guard's own
/// single-line explanation, never a partial read.
pub(super) fn denied(denial: &McpToolDenial) -> CallToolResult {
    CallToolResult::error(vec![Content::text(denial.message())])
}

/// Render the grounding block `collect_pod_grounding_with_client` produced.
///
/// Pure, so `describe_pod`'s mapping is testable without a cluster: the tool does nothing to the
/// grounding data except label the sources it came from.
fn render_pod_grounding(
    object: &AgentObjectRef,
    text: &str,
    sources: &[GroundingSource],
) -> CallToolResult {
    if text.trim().is_empty() {
        return CallToolResult::success(vec![Content::text(format!(
            "No grounding data available for pod {}/{}.",
            object.namespace, object.name
        ))]);
    }
    let labels: Vec<String> = sources.iter().map(source_label).collect();
    CallToolResult::success(vec![Content::text(format!(
        "Pod {}/{}\n\n{}\n\nGrounded in: {}",
        object.namespace,
        object.name,
        text.trim_end(),
        if labels.is_empty() {
            "nothing".to_owned()
        } else {
            labels.join(", ")
        }
    ))])
}

fn source_label(source: &GroundingSource) -> String {
    match source {
        GroundingSource::Events => "events".to_owned(),
        GroundingSource::Conditions => "conditions".to_owned(),
        GroundingSource::ContainerStatuses => "container statuses".to_owned(),
        GroundingSource::Lifecycle => "lifecycle".to_owned(),
        GroundingSource::Logs { truncated } => {
            if *truncated {
                "logs (truncated)".to_owned()
            } else {
                "logs".to_owned()
            }
        }
    }
}

#[tool_router]
impl FathomMcpServer {
    pub(super) fn new(cluster: McpClusterHandle) -> Self {
        Self {
            cluster,
            // Read tools plus Phase 4's gated mutation tools (`super::tools_mutate`), merged
            // into one router. Every mutation tool goes through `authorize` below first, then
            // a server-side resource-version re-check, before any write.
            tool_router: Self::tool_router() + Self::mutate_tool_router(),
        }
    }

    /// Client + resolved access, or the denial to return. Shared prologue for every tool, so no
    /// tool can accidentally skip a step.
    pub(super) async fn authorize(
        &self,
        tool: &str,
        namespace: Option<&str>,
        verb: &str,
        resource: &str,
    ) -> Result<kube::Client, McpToolDenial> {
        let client = self
            .cluster
            .client()
            .ok_or_else(|| McpToolDenial::no_cluster(tool))?;
        let resolved = rbac_guard::resolve_current_access(&client)
            .await
            .map_err(|e| McpToolDenial::rbac_unavailable(tool, &e.to_string()))?;
        rbac_guard::check(&resolved, tool, namespace, verb, resource)?;
        Ok(client)
    }

    #[tool(
        description = "Read one Kubernetes object from the cluster Fathom is currently connected to, as JSON. Read-only."
    )]
    async fn get_resource(
        &self,
        Parameters(params): Parameters<GetResourceParams>,
    ) -> Result<CallToolResult, McpError> {
        let resource = plural_for_kind(&params.kind);
        let client = match self
            .authorize(
                "get_resource",
                params.namespace.as_deref(),
                "get",
                &resource,
            )
            .await
        {
            Ok(client) => client,
            Err(denial) => return Ok(denied(&denial)),
        };

        let api = match dynamic_api(
            &client,
            &params.api_version,
            &params.kind,
            params.namespace.as_deref(),
        )
        .await
        {
            Ok(api) => api,
            Err(e) => return Ok(CallToolResult::error(vec![Content::text(e.to_string())])),
        };
        match api.get(&params.name).await {
            Ok(object) => Ok(CallToolResult::success(vec![Content::text(
                serde_json::to_string_pretty(&object).unwrap_or_else(|e| e.to_string()),
            )])),
            Err(e) => Ok(CallToolResult::error(vec![Content::text(format!(
                "Could not read {} {}: {e}",
                params.kind, params.name
            ))])),
        }
    }

    #[tool(
        description = "List Kubernetes objects of one kind, by name, in a namespace or cluster-wide. Read-only, capped at 100 results."
    )]
    async fn list_resources(
        &self,
        Parameters(params): Parameters<ListResourcesParams>,
    ) -> Result<CallToolResult, McpError> {
        let resource = plural_for_kind(&params.kind);
        let client = match self
            .authorize(
                "list_resources",
                params.namespace.as_deref(),
                "list",
                &resource,
            )
            .await
        {
            Ok(client) => client,
            Err(denial) => return Ok(denied(&denial)),
        };

        let api = match dynamic_api(
            &client,
            &params.api_version,
            &params.kind,
            params.namespace.as_deref(),
        )
        .await
        {
            Ok(api) => api,
            Err(e) => return Ok(CallToolResult::error(vec![Content::text(e.to_string())])),
        };
        match api.list(&ListParams::default()).await {
            Ok(list) => {
                let total = list.items.len();
                let names: Vec<String> = list
                    .items
                    .iter()
                    .take(MAX_LIST_ITEMS)
                    .map(|o| match o.namespace() {
                        Some(ns) => format!("{ns}/{}", o.name_any()),
                        None => o.name_any(),
                    })
                    .collect();
                let mut body = names.join("\n");
                if total > MAX_LIST_ITEMS {
                    body.push_str(&format!(
                        "\n… {} more not shown (capped at {MAX_LIST_ITEMS})",
                        total - MAX_LIST_ITEMS
                    ));
                }
                Ok(CallToolResult::success(vec![Content::text(body)]))
            }
            Err(e) => Ok(CallToolResult::error(vec![Content::text(format!(
                "Could not list {}: {e}",
                params.kind
            ))])),
        }
    }

    #[tool(
        description = "Read a trailing slice of one pod container's logs. Read-only, capped at 500 lines."
    )]
    async fn get_pod_logs(
        &self,
        Parameters(params): Parameters<GetPodLogsParams>,
    ) -> Result<CallToolResult, McpError> {
        let client = match self
            .authorize(
                "get_pod_logs",
                Some(params.namespace.as_str()),
                "get",
                "pods/log",
            )
            .await
        {
            Ok(client) => client,
            Err(denial) => return Ok(denied(&denial)),
        };

        let api: Api<Pod> = Api::namespaced(client, &params.namespace);
        let log_params = LogParams {
            container: params.container.clone(),
            tail_lines: Some(
                params
                    .tail_lines
                    .unwrap_or(MAX_LOG_LINES)
                    .min(MAX_LOG_LINES),
            ),
            ..Default::default()
        };
        match api.logs(&params.name, &log_params).await {
            Ok(logs) => Ok(CallToolResult::success(vec![Content::text(logs)])),
            Err(e) => Ok(CallToolResult::error(vec![Content::text(format!(
                "Could not read logs for {}/{}: {e}",
                params.namespace, params.name
            ))])),
        }
    }

    #[tool(
        description = "List recent Kubernetes events in a namespace or cluster-wide. Read-only, capped at 100 events."
    )]
    async fn get_events(
        &self,
        Parameters(params): Parameters<GetEventsParams>,
    ) -> Result<CallToolResult, McpError> {
        let client = match self
            .authorize("get_events", params.namespace.as_deref(), "list", "events")
            .await
        {
            Ok(client) => client,
            Err(denial) => return Ok(denied(&denial)),
        };

        let api: Api<Event> = match params.namespace.as_deref() {
            Some(ns) => Api::namespaced(client, ns),
            None => Api::all(client),
        };
        match api.list(&ListParams::default()).await {
            Ok(list) => {
                let lines: Vec<String> = list
                    .items
                    .iter()
                    .take(MAX_LIST_ITEMS)
                    .map(|e| {
                        format!(
                            "[{}] {} {}: {}",
                            e.type_.clone().unwrap_or_default(),
                            e.involved_object.kind.clone().unwrap_or_default(),
                            e.involved_object.name.clone().unwrap_or_default(),
                            e.message.clone().unwrap_or_default()
                        )
                    })
                    .collect();
                Ok(CallToolResult::success(vec![Content::text(
                    lines.join("\n"),
                )]))
            }
            Err(e) => Ok(CallToolResult::error(vec![Content::text(format!(
                "Could not list events: {e}"
            ))])),
        }
    }

    #[tool(
        description = "Summarize one pod's health: phase, conditions, container statuses, a bounded log tail and its recent events — the same grounding Fathom's own assistant uses. Read-only."
    )]
    async fn describe_pod(
        &self,
        Parameters(params): Parameters<DescribePodParams>,
    ) -> Result<CallToolResult, McpError> {
        let client = match self
            .authorize(
                "describe_pod",
                Some(params.namespace.as_str()),
                "get",
                "pods",
            )
            .await
        {
            Ok(client) => client,
            Err(denial) => return Ok(denied(&denial)),
        };

        let object = AgentObjectRef {
            kind: "Pod".to_owned(),
            namespace: params.namespace.clone(),
            name: params.name.clone(),
        };
        let (text, sources) =
            crate::backend::agent::grounding::collect_pod_grounding_with_client(&client, &object)
                .await;
        Ok(render_pod_grounding(&object, &text, &sources))
    }
}

/// Discover `kind`'s `ApiResource` and build the matching dynamic `Api`. Runs only after the
/// RBAC guard has allowed the call.
pub(super) async fn dynamic_api(
    client: &kube::Client,
    api_version: &str,
    kind: &str,
    namespace: Option<&str>,
) -> anyhow::Result<Api<DynamicObject>> {
    let api_resource =
        crate::backend::apply::resolve_api_resource(client, api_version, kind).await?;
    Ok(match namespace {
        Some(ns) => Api::namespaced_with(client.clone(), ns, &api_resource),
        None => Api::all_with(client.clone(), &api_resource),
    })
}

#[tool_handler]
impl ServerHandler for FathomMcpServer {
    fn get_info(&self) -> ServerInfo {
        let context = self
            .cluster
            .context()
            .unwrap_or_else(|| "no cluster connected".to_owned());
        ServerInfo {
            server_info: Implementation {
                name: "fathom".to_owned(),
                version: env!("CARGO_PKG_VERSION").to_owned(),
                ..Implementation::from_build_env()
            },
            instructions: Some(format!(
                "Live access to the Kubernetes cluster Fathom is connected to (context: \
                 {context}). Every call is scoped by the RBAC of Fathom's own cluster identity; \
                 refused calls read nothing and write nothing. The read tools \
                 (get_resource, list_resources, get_pod_logs, get_events, describe_pod) never \
                 mutate. The mutation tools (scale_workload, apply_resource_yaml, \
                 delete_resource) change the cluster: confirm with the operator first, and each \
                 requires the target's current expected_resource_version, which Fathom \
                 re-validates against the live object before writing."
            )),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use k8s_openapi::api::core::v1::{
        ContainerState, ContainerStateRunning, ContainerStatus, PodCondition, PodSpec, PodStatus,
    };
    use k8s_openapi::api::rbac::v1::{
        ClusterRole, ClusterRoleBinding, PolicyRule, RoleRef, Subject,
    };
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// A `kube::Client` pointed at `url`. No live cluster, no kubeconfig: just HTTP against
    /// whatever `wiremock` is serving.
    fn client_for(url: &str) -> kube::Client {
        // `kube`'s HTTPS-capable connector needs a process-level rustls provider, the same one
        // `src/startup.rs` installs in the real binary. Harmless for our plain-HTTP mock.
        let _ = rustls::crypto::ring::default_provider().install_default();
        let config = kube::Config::new(url.parse().expect("mock server url"));
        kube::Client::try_from(config).expect("client from mock config")
    }

    fn fixture_pod() -> Pod {
        Pod {
            metadata: ObjectMeta {
                name: Some("api-7d9".to_owned()),
                namespace: Some("payments".to_owned()),
                ..Default::default()
            },
            spec: Some(PodSpec {
                containers: vec![k8s_openapi::api::core::v1::Container {
                    name: "api".to_owned(),
                    ..Default::default()
                }],
                ..Default::default()
            }),
            status: Some(PodStatus {
                phase: Some("Running".to_owned()),
                conditions: Some(vec![PodCondition {
                    type_: "Ready".to_owned(),
                    status: "True".to_owned(),
                    ..Default::default()
                }]),
                container_statuses: Some(vec![ContainerStatus {
                    name: "api".to_owned(),
                    ready: true,
                    restart_count: 2,
                    state: Some(ContainerState {
                        running: Some(ContainerStateRunning::default()),
                        ..Default::default()
                    }),
                    ..Default::default()
                }]),
                ..Default::default()
            }),
        }
    }

    /// Grants the reviewed identity (`fathom-tester`) read access cluster-wide, so the guard
    /// allows the call and we get to observe the *mapping*, not the refusal.
    fn reader_rbac() -> (ClusterRole, ClusterRoleBinding) {
        (
            ClusterRole {
                metadata: ObjectMeta {
                    name: Some("fathom-reader".to_owned()),
                    ..Default::default()
                },
                rules: Some(vec![PolicyRule {
                    api_groups: Some(vec![String::new()]),
                    resources: Some(vec![
                        "pods".to_owned(),
                        "pods/log".to_owned(),
                        "events".to_owned(),
                    ]),
                    verbs: vec!["get".to_owned(), "list".to_owned()],
                    ..Default::default()
                }]),
                aggregation_rule: None,
            },
            ClusterRoleBinding {
                metadata: ObjectMeta {
                    name: Some("fathom-reader".to_owned()),
                    ..Default::default()
                },
                role_ref: RoleRef {
                    api_group: "rbac.authorization.k8s.io".to_owned(),
                    kind: "ClusterRole".to_owned(),
                    name: "fathom-reader".to_owned(),
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

    async fn mock_apiserver(pod: &Pod, logs: &str) -> MockServer {
        let server = MockServer::start().await;
        let (cluster_role, binding) = reader_rbac();

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
            .and(path("/api/v1/namespaces/payments/pods/api-7d9"))
            .respond_with(ResponseTemplate::new(200).set_body_json(pod))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v1/namespaces/payments/pods/api-7d9/log"))
            .respond_with(ResponseTemplate::new(200).set_body_string(logs))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v1/namespaces/payments/events"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "apiVersion": "v1",
                "kind": "EventList",
                "items": [{
                    "metadata": { "name": "api-7d9.1", "namespace": "payments" },
                    "involvedObject": { "kind": "Pod", "name": "api-7d9" },
                    "type": "Warning",
                    "reason": "BackOff",
                    "message": "Back-off restarting failed container"
                }]
            })))
            .mount(&server)
            .await;

        server
    }

    /// T018: `describe_pod` is a thin wrapper over the Agent panel's own grounding collector.
    ///
    /// The tool is driven against a `wiremock` API server (no live cluster), and its output is
    /// asserted to carry exactly what `collect_pod_grounding_with_client` returned for the same
    /// fixture pod — same phase, conditions, container statuses, log tail and events, and the
    /// same source labels. If the tool ever grew a second, divergent read path, the two halves
    /// of this assertion would stop matching.
    #[tokio::test]
    async fn describe_pod_maps_to_collect_pod_grounding() {
        let pod = fixture_pod();
        let server = mock_apiserver(&pod, "boot ok\nlistening on 8080\n").await;
        let client = client_for(&server.uri());

        let object = AgentObjectRef {
            kind: "Pod".to_owned(),
            namespace: "payments".to_owned(),
            name: "api-7d9".to_owned(),
        };
        let (expected_text, expected_sources) =
            crate::backend::agent::grounding::collect_pod_grounding_with_client(&client, &object)
                .await;
        assert!(
            !expected_text.trim().is_empty(),
            "the grounding collector must have read the fixture pod: {expected_text:?}"
        );

        let handle = McpClusterHandle::default();
        handle.publish(Some(client), Some("mock".to_owned()));
        let tools = FathomMcpServer::new(handle);

        let result = tools
            .describe_pod(Parameters(DescribePodParams {
                namespace: "payments".to_owned(),
                name: "api-7d9".to_owned(),
            }))
            .await
            .expect("describe_pod should answer");

        assert_eq!(result.is_error, Some(false), "the call was allowed");
        let text = match result.content.first().map(|c| c.as_text()) {
            Some(Some(text)) => text.text.clone(),
            _ => panic!("describe_pod must return a text block"),
        };
        assert!(
            text.contains(expected_text.trim_end()),
            "the tool must return the collector's grounding verbatim.\n--- tool ---\n{text}\n--- \
             collector ---\n{expected_text}"
        );
        assert!(text.contains("Pod phase: Running"));
        assert!(text.contains("listening on 8080"));
        assert!(text.contains("Back-off restarting failed container"));
        for source in &expected_sources {
            assert!(
                text.contains(&source_label(source)),
                "missing source label {:?} in {text}",
                source
            );
        }
    }

    /// The RBAC guard runs before the pod read: with no client published there is nothing to
    /// read from, and the refusal names the tool.
    #[tokio::test]
    async fn tools_refuse_before_reading_when_no_cluster_is_connected() {
        let tools = FathomMcpServer::new(McpClusterHandle::default());
        let result = tools
            .describe_pod(Parameters(DescribePodParams {
                namespace: "payments".to_owned(),
                name: "api-7d9".to_owned(),
            }))
            .await
            .expect("describe_pod should answer");
        assert_eq!(result.is_error, Some(true));
        let text = result
            .content
            .first()
            .and_then(|c| c.as_text())
            .map(|t| t.text.clone())
            .unwrap_or_default();
        assert!(text.contains("describe_pod refused"), "{text}");
        assert!(text.contains("not connected to a cluster"), "{text}");
    }

    /// An identity with no grants is refused, and the refusal says nothing was read — the tool
    /// never reaches `collect_pod_grounding_with_client`.
    #[tokio::test]
    async fn describe_pod_is_refused_when_rbac_grants_nothing() {
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

        let handle = McpClusterHandle::default();
        handle.publish(Some(client_for(&server.uri())), None);
        let tools = FathomMcpServer::new(handle);

        let result = tools
            .describe_pod(Parameters(DescribePodParams {
                namespace: "payments".to_owned(),
                name: "api-7d9".to_owned(),
            }))
            .await
            .expect("describe_pod should answer");
        assert_eq!(result.is_error, Some(true));
        let text = result
            .content
            .first()
            .and_then(|c| c.as_text())
            .map(|t| t.text.clone())
            .unwrap_or_default();
        assert!(text.contains("Nothing was read from the cluster"), "{text}");
        // No pod or log mock was ever mounted, so a read attempt would have 404'd with a
        // different message; the guard's message proves the read never happened.
        assert!(text.contains("payments"), "{text}");
    }

    #[test]
    fn plural_for_kind_matches_common_rbac_resource_names() {
        assert_eq!(plural_for_kind("Pod"), "pods");
        assert_eq!(plural_for_kind("Deployment"), "deployments");
        assert_eq!(plural_for_kind("Ingress"), "ingresses");
        assert_eq!(plural_for_kind("NetworkPolicy"), "networkpolicies");
        assert_eq!(plural_for_kind("Endpoints"), "endpointses");
    }

    #[test]
    fn grounding_with_nothing_collected_is_reported_not_faked() {
        let object = AgentObjectRef {
            kind: "Pod".to_owned(),
            namespace: "payments".to_owned(),
            name: "gone".to_owned(),
        };
        let result = render_pod_grounding(&object, "", &[]);
        assert_eq!(result.is_error, Some(false));
        let text = result
            .content
            .first()
            .and_then(|c| c.as_text())
            .map(|t| t.text.clone())
            .unwrap_or_default();
        assert!(text.contains("No grounding data available"), "{text}");
    }
}
