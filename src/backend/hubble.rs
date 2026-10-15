//! Hubble/Cilium eBPF traffic-observability integration.
//!
//! Discovers a running `hubble-relay` Service, keeps one port-forward open to
//! its gRPC port (4245) across polls (see [`HubbleSampler`]), and samples a
//! bounded number of recent flows via `Observer.GetFlows` (`follow: false`,
//! so each RPC completes on its own). Mirrors the discovery shape of
//! `flux.rs`, but there is no CRD/dynamic-object listing here: Hubble Relay
//! is a plain Service + gRPC API, not a CRD-backed resource.

use crate::protocol::{HubbleDropReason, HubbleFlowPair, HubbleTrafficSummary, HubbleVerdict};
use k8s_openapi::api::core::v1::{Pod, Service};
use kube::api::ListParams;
use kube::{Client, api::Api};

#[allow(
    dead_code,
    clippy::large_enum_variant,
    clippy::too_many_lines,
    clippy::enum_variant_names
)]
mod proto {
    pub mod observer {
        tonic::include_proto!("observer");
    }
    pub mod flow {
        tonic::include_proto!("flow");
    }
    pub mod relay {
        tonic::include_proto!("relay");
    }
}

use proto::observer::observer_client::ObserverClient;
use proto::observer::{GetFlowsRequest, get_flows_response::ResponseTypes};

const RELAY_SERVICE_NAME: &str = "hubble-relay";
const RELAY_LABEL_SELECTOR: &str = "k8s-app=hubble-relay";
const RELAY_GRPC_PORT: u16 = 4245;
const SAMPLE_SIZE: u64 = 200;
const TOP_N: usize = 5;

pub(super) struct HubbleConfig {
    pub(super) relay_namespace: String,
}

/// Look for a Service named `hubble-relay` in any namespace. Its presence is
/// sufficient signal that Hubble Relay's gRPC API is reachable; Cilium can be
/// installed without every Cilium CRD present, so this doesn't gate on CRDs.
pub(super) async fn detect_hubble(client: &Client) -> Option<HubbleConfig> {
    let svc_api: Api<Service> = Api::all(client.clone());
    let svcs = svc_api.list(&ListParams::default()).await.ok()?;
    svcs.items
        .into_iter()
        .find(|s| s.metadata.name.as_deref() == Some(RELAY_SERVICE_NAME))
        .and_then(|s| {
            let namespace = s.metadata.namespace?;
            Some(HubbleConfig {
                relay_namespace: namespace,
            })
        })
}

/// A single flow reduced to just what `aggregate` needs, so the aggregation
/// logic is unit-testable without a gRPC connection.
struct FlowRecord {
    verdict: HubbleVerdict,
    source_namespace: String,
    dest_namespace: String,
    drop_reason: Option<String>,
}

fn map_verdict(v: proto::flow::Verdict) -> HubbleVerdict {
    match v {
        proto::flow::Verdict::Forwarded | proto::flow::Verdict::Redirected => {
            HubbleVerdict::Forwarded
        }
        proto::flow::Verdict::Dropped => HubbleVerdict::Dropped,
        proto::flow::Verdict::Audit => HubbleVerdict::Audit,
        proto::flow::Verdict::Error => HubbleVerdict::Error,
        proto::flow::Verdict::Unknown
        | proto::flow::Verdict::Traced
        | proto::flow::Verdict::Translated => HubbleVerdict::Other,
    }
}

/// Build a [`HubbleTrafficSummary`] from a flat list of sampled flows.
fn aggregate(flows: &[FlowRecord], generated_at: String) -> HubbleTrafficSummary {
    let total_flows = flows.len() as u32;
    let forwarded = flows
        .iter()
        .filter(|f| f.verdict == HubbleVerdict::Forwarded)
        .count() as u32;
    let dropped = flows
        .iter()
        .filter(|f| f.verdict == HubbleVerdict::Dropped)
        .count() as u32;

    let mut pair_counts: std::collections::HashMap<(String, String), u32> =
        std::collections::HashMap::new();
    for f in flows {
        *pair_counts
            .entry((f.source_namespace.clone(), f.dest_namespace.clone()))
            .or_insert(0) += 1;
    }
    let mut top_pairs: Vec<HubbleFlowPair> = pair_counts
        .into_iter()
        .map(
            |((source_namespace, dest_namespace), count)| HubbleFlowPair {
                source_namespace,
                dest_namespace,
                count,
            },
        )
        .collect();
    top_pairs.sort_by(|a, b| {
        b.count.cmp(&a.count).then_with(|| {
            (a.source_namespace.as_str(), a.dest_namespace.as_str())
                .cmp(&(b.source_namespace.as_str(), b.dest_namespace.as_str()))
        })
    });
    top_pairs.truncate(TOP_N);

    let mut reason_counts: std::collections::HashMap<String, u32> =
        std::collections::HashMap::new();
    for f in flows.iter().filter_map(|f| f.drop_reason.as_ref()) {
        *reason_counts.entry(f.clone()).or_insert(0) += 1;
    }
    let mut top_drop_reasons: Vec<HubbleDropReason> = reason_counts
        .into_iter()
        .map(|(reason, count)| HubbleDropReason { reason, count })
        .collect();
    top_drop_reasons.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.reason.cmp(&b.reason)));
    top_drop_reasons.truncate(TOP_N);

    HubbleTrafficSummary {
        installed: true,
        generated_at,
        total_flows,
        forwarded,
        dropped,
        top_pairs,
        top_drop_reasons,
    }
}

/// One port-forward to the `hubble-relay` pod and the gRPC channel over it.
///
/// The background poll keeps one of these for as long as the relay answers,
/// so a session opens a single port-forward instead of one per poll: every
/// open is a `pods/portforward` call that the API server audits. HTTP/2
/// multiplexes each `GetFlows` RPC over the same stream.
struct RelayConnection {
    forwarder: kube::api::Portforwarder,
    observer: ObserverClient<tonic::transport::Channel>,
}

impl Drop for RelayConnection {
    // Dropping the forwarder's JoinHandle would leave its task (and the
    // websocket to the API server) running, so abort it explicitly.
    fn drop(&mut self) {
        self.forwarder.abort();
    }
}

/// Open a port-forward to the `hubble-relay` pod and a gRPC channel over it.
/// `Ok(None)` when the relay Service exists but no pod backs it yet.
async fn connect_relay(
    client: &Client,
    cfg: &HubbleConfig,
) -> anyhow::Result<Option<RelayConnection>> {
    let pod_api: Api<Pod> = Api::namespaced(client.clone(), &cfg.relay_namespace);
    let pods = pod_api
        .list(&ListParams::default().labels(RELAY_LABEL_SELECTOR))
        .await?;
    let Some(pod_name) = pods.items.into_iter().find_map(|p| p.metadata.name) else {
        return Ok(None);
    };

    let mut forwarder = pod_api.portforward(&pod_name, &[RELAY_GRPC_PORT]).await?;
    let Some(stream) = forwarder.take_stream(RELAY_GRPC_PORT) else {
        forwarder.abort();
        return Ok(None);
    };

    // Hand tonic the already-open portforward stream directly instead of a local
    // TCP socket. The connector yields it once: if the channel ever needs to
    // reconnect, the RPC fails and the caller opens a new `RelayConnection`.
    let mut stream_opt = Some(hyper_util::rt::TokioIo::new(stream));
    let channel = match tonic::transport::Endpoint::try_from("http://[::]:0")?
        .connect_with_connector(tower::service_fn(move |_| {
            let stream = stream_opt.take();
            async move {
                stream.ok_or_else(|| {
                    std::io::Error::other("hubble portforward stream already consumed")
                })
            }
        }))
        .await
    {
        Ok(channel) => channel,
        Err(e) => {
            forwarder.abort();
            return Err(e.into());
        }
    };

    Ok(Some(RelayConnection {
        forwarder,
        observer: ObserverClient::new(channel),
    }))
}

/// Sample up to [`SAMPLE_SIZE`] recent flows via `Observer.GetFlows` over an
/// open relay connection and aggregate them into a [`HubbleTrafficSummary`].
async fn sample_traffic(conn: &mut RelayConnection) -> anyhow::Result<HubbleTrafficSummary> {
    let generated_at = chrono::Utc::now().to_rfc3339();
    let request = GetFlowsRequest {
        number: SAMPLE_SIZE,
        follow: false,
        ..Default::default()
    };
    let mut response = conn.observer.get_flows(request).await?.into_inner();

    let mut flows = Vec::new();
    while let Some(item) = futures_util::StreamExt::next(&mut response).await {
        let Ok(resp) = item else { break };
        if let Some(ResponseTypes::Flow(flow)) = resp.response_types {
            let verdict = proto::flow::Verdict::try_from(flow.verdict)
                .map(map_verdict)
                .unwrap_or(HubbleVerdict::Other);
            let drop_reason = if verdict == HubbleVerdict::Dropped {
                proto::flow::DropReason::try_from(flow.drop_reason_desc)
                    .ok()
                    .map(|r| r.as_str_name().to_owned())
            } else {
                None
            };
            flows.push(FlowRecord {
                verdict,
                source_namespace: flow.source.map(|e| e.namespace).unwrap_or_default(),
                dest_namespace: flow.destination.map(|e| e.namespace).unwrap_or_default(),
                drop_reason,
            });
        }
    }

    Ok(aggregate(&flows, generated_at))
}

/// Summary for a relay Service with no pod behind it yet.
fn no_relay_pod() -> HubbleTrafficSummary {
    HubbleTrafficSummary {
        installed: true,
        generated_at: chrono::Utc::now().to_rfc3339(),
        ..Default::default()
    }
}

/// One-shot sample for an explicit refresh: open a connection, sample, close.
/// The background poll uses [`HubbleSampler`] instead, which keeps one open.
pub(super) async fn fetch_traffic_summary(
    client: &Client,
    cfg: &HubbleConfig,
) -> anyhow::Result<HubbleTrafficSummary> {
    match connect_relay(client, cfg).await? {
        Some(mut conn) => sample_traffic(&mut conn).await,
        None => Ok(no_relay_pod()),
    }
}

/// Keeps one relay connection across polls and reopens it only when it fails.
#[derive(Default)]
pub(super) struct HubbleSampler {
    conn: Option<RelayConnection>,
}

impl HubbleSampler {
    pub(super) async fn sample(
        &mut self,
        client: &Client,
        cfg: &HubbleConfig,
    ) -> anyhow::Result<HubbleTrafficSummary> {
        if let Some(conn) = self.conn.as_mut() {
            match sample_traffic(conn).await {
                Ok(summary) => return Ok(summary),
                // A reused connection going stale (relay pod restarted, API server
                // dropped the stream) is expected: reconnect once before reporting.
                Err(e) => tracing::debug!("hubble relay connection lost, reconnecting: {e:#}"),
            }
        }
        self.conn = None;
        let Some(mut conn) = connect_relay(client, cfg).await? else {
            return Ok(no_relay_pod());
        };
        let summary = sample_traffic(&mut conn).await?;
        self.conn = Some(conn);
        Ok(summary)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(
        verdict: HubbleVerdict,
        source_namespace: &str,
        dest_namespace: &str,
        drop_reason: Option<&str>,
    ) -> FlowRecord {
        FlowRecord {
            verdict,
            source_namespace: source_namespace.to_owned(),
            dest_namespace: dest_namespace.to_owned(),
            drop_reason: drop_reason.map(str::to_owned),
        }
    }

    #[test]
    fn aggregate_counts_verdicts() {
        let flows = vec![
            record(HubbleVerdict::Forwarded, "a", "b", None),
            record(HubbleVerdict::Forwarded, "a", "b", None),
            record(HubbleVerdict::Dropped, "a", "c", Some("POLICY_DENIED")),
        ];
        let summary = aggregate(&flows, "now".to_owned());
        assert_eq!(summary.total_flows, 3);
        assert_eq!(summary.forwarded, 2);
        assert_eq!(summary.dropped, 1);
        assert!(summary.installed);
    }

    #[test]
    fn aggregate_ranks_top_pairs_desc_with_tiebreak() {
        let flows = vec![
            record(HubbleVerdict::Forwarded, "a", "z", None),
            record(HubbleVerdict::Forwarded, "b", "y", None),
            record(HubbleVerdict::Forwarded, "b", "y", None),
        ];
        let summary = aggregate(&flows, "now".to_owned());
        assert_eq!(summary.top_pairs[0].source_namespace, "b");
        assert_eq!(summary.top_pairs[0].count, 2);
        assert_eq!(summary.top_pairs[1].source_namespace, "a");
    }

    #[test]
    fn aggregate_truncates_to_top_n() {
        let flows: Vec<FlowRecord> = (0..10)
            .map(|i| {
                record(
                    HubbleVerdict::Dropped,
                    "a",
                    &format!("ns{i}"),
                    Some("POLICY_DENIED"),
                )
            })
            .collect();
        let summary = aggregate(&flows, "now".to_owned());
        assert_eq!(summary.top_pairs.len(), TOP_N);
        assert_eq!(summary.top_drop_reasons.len(), 1);
        assert_eq!(summary.top_drop_reasons[0].count, 10);
    }
}
