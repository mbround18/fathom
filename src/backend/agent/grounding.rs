//! Live, on-demand grounding data collection (T013).
//!
//! There is no existing in-memory queryable cache of pod/event data in this backend —
//! `src/backend/pods.rs`'s handlers fetch live via `state.cluster.client_or_err()?` →
//! `Api<Pod>::all(client).list(...)` per request, and `LogStreamState` is a live
//! streaming task handle, not a readable buffer. So this module fetches fresh data at
//! `AgentAsk` time via the same kube client `BackendState` already holds — the same
//! `Api<Pod>`/`Api<Event>`/`api.logs(...)` surface every other handler in this backend
//! already uses, just called synchronously here instead of from a cache. This is not a
//! new ingestion/watch/polling path (FR-001): it is a bounded, one-shot read per ask.
//!
//! Output is truncated per Principle V's bounded-data rule: at most
//! `MAX_EVENTS` events and `MAX_LOG_LINES` trailing log lines are included, and a
//! truncated log tail is reported via `GroundingSource::Logs { truncated: true }`
//! (FR-018).

use super::redact;
use crate::backend::state::BackendState;
use crate::protocol::v1::{AgentObjectRef, GroundingSource};
use k8s_openapi::api::core::v1::{Event, Pod};
use kube::api::{Api, ListParams, LogParams};

const MAX_EVENTS: usize = 20;
const MAX_LOG_LINES: i64 = 200;

/// Collects grounding context for `object` and renders it as a plain-text block
/// suitable for inclusion in the system prompt, plus the list of sources actually
/// used (so the frontend can render "grounded in: events, logs, ..." chips, FR-012).
pub(crate) async fn collect_pod_grounding(
    state: &BackendState,
    object: &AgentObjectRef,
) -> (String, Vec<GroundingSource>) {
    let Ok(client) = state.cluster.client_or_err() else {
        return (String::new(), Vec::new());
    };
    collect_pod_grounding_with_client(&client, object).await
}

/// The same collection logic, against an explicitly supplied client rather than whatever
/// `BackendState` currently holds.
///
/// Split out for 015-mcp-agent-exposure's `describe_pod` MCP tool
/// (`crate::backend::mcp::tools_read`), which serves external MCP clients from a spawned task
/// that has a `kube::Client` but no `&BackendState` borrow. Reusing this exact function is what
/// makes the MCP tool's output identical to the in-app Agent panel's grounding (research.md R6);
/// it is not a second, parallel read path.
pub(crate) async fn collect_pod_grounding_with_client(
    client: &kube::Client,
    object: &AgentObjectRef,
) -> (String, Vec<GroundingSource>) {
    if object.kind != "Pod" {
        // Only Pod grounding is implemented for the US1 MVP slice; other kinds get an
        // empty grounding block rather than a fabricated one.
        return (String::new(), Vec::new());
    }

    let mut sections = Vec::new();
    let mut sources = Vec::new();

    let pod_api: Api<Pod> = Api::namespaced(client.clone(), &object.namespace);
    if let Ok(pod) = pod_api.get(&object.name).await {
        if let Some(status) = &pod.status {
            if let Some(phase) = &status.phase {
                sections.push(format!("Pod phase: {phase}"));
            }
            if let Some(conditions) = &status.conditions {
                let cond_lines: Vec<String> = conditions
                    .iter()
                    .map(|c| {
                        format!(
                            "- {} = {} ({})",
                            c.type_,
                            c.status,
                            c.reason.clone().unwrap_or_default()
                        )
                    })
                    .collect();
                if !cond_lines.is_empty() {
                    sections.push(format!("Conditions:\n{}", cond_lines.join("\n")));
                    sources.push(GroundingSource::Conditions);
                }
            }
            if let Some(statuses) = &status.container_statuses {
                let cs_lines: Vec<String> = statuses
                    .iter()
                    .map(|cs| {
                        let state_desc = describe_container_state(&cs.state);
                        format!(
                            "- {}: ready={} restartCount={} state={}",
                            cs.name, cs.ready, cs.restart_count, state_desc
                        )
                    })
                    .collect();
                if !cs_lines.is_empty() {
                    sections.push(format!("Container statuses:\n{}", cs_lines.join("\n")));
                    sources.push(GroundingSource::ContainerStatuses);
                    sources.push(GroundingSource::Lifecycle);
                }
            }
        }

        // Bounded log tail for the first container (best-effort; missing logs are not
        // an error worth surfacing to the operator, just omitted).
        if let Some(spec) = &pod.spec
            && let Some(first_container) = spec.containers.first()
        {
            let log_params = LogParams {
                tail_lines: Some(MAX_LOG_LINES),
                container: Some(first_container.name.clone()),
                ..Default::default()
            };
            if let Ok(logs) = pod_api.logs(&object.name, &log_params).await {
                let line_count = logs.lines().count();
                let truncated = line_count as i64 >= MAX_LOG_LINES;
                if !logs.trim().is_empty() {
                    // Scrubbed before it can reach a hosted provider (see `redact.rs`).
                    let logs = redact::redact_text(&logs);
                    sections.push(format!("Recent logs ({}):\n{}", first_container.name, logs));
                    sources.push(GroundingSource::Logs { truncated });
                }
            }
        }
    }

    // Events involving this pod, bounded to MAX_EVENTS most recent.
    let event_api: Api<Event> = Api::namespaced(client.clone(), &object.namespace);
    if let Ok(list) = event_api.list(&ListParams::default()).await {
        let mut matching: Vec<_> = list
            .items
            .into_iter()
            .filter(|e| {
                e.involved_object
                    .name
                    .as_deref()
                    .map(|n| n == object.name)
                    .unwrap_or(false)
            })
            .collect();
        matching.truncate(MAX_EVENTS);
        if !matching.is_empty() {
            let lines: Vec<String> = matching
                .iter()
                .map(|e| {
                    format!(
                        "- [{}] {}: {}",
                        e.type_.clone().unwrap_or_default(),
                        e.reason.clone().unwrap_or_default(),
                        redact::redact_line(e.message.as_deref().unwrap_or_default())
                    )
                })
                .collect();
            sections.push(format!("Events:\n{}", lines.join("\n")));
            sources.push(GroundingSource::Events);
        }
    }

    (sections.join("\n\n"), sources)
}

fn describe_container_state(state: &Option<k8s_openapi::api::core::v1::ContainerState>) -> String {
    let Some(state) = state else {
        return "unknown".to_owned();
    };
    if let Some(waiting) = &state.waiting {
        format!("waiting({})", waiting.reason.clone().unwrap_or_default())
    } else if let Some(running) = &state.running {
        format!(
            "running(since {})",
            running
                .started_at
                .clone()
                .map(|t| t.0.to_rfc3339())
                .unwrap_or_default()
        )
    } else if let Some(terminated) = &state.terminated {
        format!(
            "terminated(exitCode={}, reason={})",
            terminated.exit_code,
            terminated.reason.clone().unwrap_or_default()
        )
    } else {
        "unknown".to_owned()
    }
}
