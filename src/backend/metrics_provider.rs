/// Metrics provider abstraction and Prometheus client implementation
use anyhow::Result;
use std::collections::BTreeMap;

use crate::protocol::{
    CpuMetrics, InstantQueryResponse, MemoryMetrics, MetricValue, NetworkMetrics, PrometheusMetrics,
};

/// Trait for metrics providers (Prometheus, Thanos, etc.)
#[async_trait::async_trait]
pub trait MetricsProviderTrait {
    /// Query a PromQL expression at a specific point in time
    async fn query_instant(&self, query: &str, time: Option<u64>) -> Result<InstantQueryResponse>;

    /// Query a PromQL expression over a time range
    #[allow(dead_code)] // planned API: range queries for metrics history graphs
    async fn query_range(
        &self,
        query: &str,
        start: u64,
        end: u64,
        step: &str,
    ) -> Result<serde_json::Value>;

    /// Get available metrics metadata
    #[allow(dead_code)] // planned API: metric discovery for the metrics integration
    async fn get_metadata(
        &self,
        metric_name: Option<&str>,
    ) -> Result<BTreeMap<String, Vec<MetricMetadata>>>;
}

#[allow(dead_code)] // planned API: returned by `MetricsProviderTrait::get_metadata`
#[derive(Debug, Clone)]
pub struct MetricMetadata {
    pub metric_type: String,
    pub help: String,
    pub unit: String,
}

/// Where Prometheus-compatible HTTP API requests go (041-observability-autoconnect).
///
/// `ServiceProxy` reaches an in-cluster Service through the Kubernetes API server's service
/// proxy (`/api/v1/namespaces/{ns}/services/{svc}:{port}/proxy/...`) using the active context's
/// own credentials, so a detected Prometheus answers queries with no port-forward and no user
/// input. `Url` is a plain base URL (a manual `localhost` forward, or any reachable address).
#[derive(Clone)]
pub enum PromEndpoint {
    Url(String),
    ServiceProxy {
        client: kube::Client,
        namespace: String,
        service: String,
        port: u16,
    },
}

/// Request timeout for a single Prometheus API call.
const PROM_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

impl PromEndpoint {
    /// Human-readable target, for status messages and logs.
    pub fn describe(&self) -> String {
        match self {
            Self::Url(url) => url.clone(),
            Self::ServiceProxy {
                namespace,
                service,
                port,
                ..
            } => format!("{namespace}/{service}:{port} via API server proxy"),
        }
    }

    /// GET `path_and_query` (starting with `/api/v1/...`) and return the body.
    pub async fn get_text(&self, path_and_query: &str) -> Result<String> {
        match self {
            Self::Url(base) => {
                let url = format!("{}{}", base.trim_end_matches('/'), path_and_query);
                let resp = crate::backend::http_client::shared()
                    .get(&url)
                    .timeout(PROM_TIMEOUT)
                    .send()
                    .await?;
                let status = resp.status();
                let body = resp.text().await?;
                if !status.is_success() {
                    anyhow::bail!("Prometheus returned {status}: {}", truncate(&body));
                }
                Ok(body)
            }
            Self::ServiceProxy {
                client,
                namespace,
                service,
                port,
            } => {
                let path = service_proxy_path(namespace, service, *port, path_and_query);
                let req = http::Request::get(path).body(Vec::new())?;
                match tokio::time::timeout(PROM_TIMEOUT, client.request_text(req)).await {
                    Ok(Ok(body)) => Ok(body),
                    Ok(Err(e)) => Err(anyhow::anyhow!("API server proxy request failed: {e}")),
                    Err(_) => anyhow::bail!("API server proxy request timed out"),
                }
            }
        }
    }

    /// GET `path_and_query` and parse the body as JSON.
    pub async fn get_json(&self, path_and_query: &str) -> Result<serde_json::Value> {
        let body = self.get_text(path_and_query).await?;
        serde_json::from_str(&body)
            .map_err(|e| anyhow::anyhow!("Invalid Prometheus response ({e}): {}", truncate(&body)))
    }

    /// Cheap reachability check: an instant `up` query that must report `status: success`.
    pub async fn probe(&self) -> Result<()> {
        let json = self.get_json("/api/v1/query?query=up").await?;
        match json.get("status").and_then(|s| s.as_str()) {
            Some("success") => Ok(()),
            other => anyhow::bail!("Prometheus returned status {:?}", other),
        }
    }
}

/// The API server's service-proxy path for `path_and_query` on `namespace/service:port`.
pub fn service_proxy_path(
    namespace: &str,
    service: &str,
    port: u16,
    path_and_query: &str,
) -> String {
    format!("/api/v1/namespaces/{namespace}/services/{service}:{port}/proxy{path_and_query}")
}

fn truncate(body: &str) -> &str {
    let end = body
        .char_indices()
        .nth(200)
        .map(|(i, _)| i)
        .unwrap_or(body.len());
    &body[..end]
}

/// Parse a Prometheus sample timestamp. The HTTP API returns a float of seconds
/// (`1790448285.378`), never an integer, so an `as_u64` read drops every sample.
pub fn parse_sample_timestamp(v: &serde_json::Value) -> Option<u64> {
    v.as_f64()
        .filter(|t| t.is_finite() && *t >= 0.0)
        .map(|t| t as u64)
}

/// Parse a Prometheus sample value (a string that may be a float, scientific notation,
/// `NaN` or `+Inf`) as a non-negative integer, rounding. Non-finite values are `None`.
pub fn parse_sample_u64(s: &str) -> Option<u64> {
    s.parse::<f64>()
        .ok()
        .filter(|v| v.is_finite() && *v >= 0.0)
        .map(|v| v.round() as u64)
}

/// Prometheus HTTP API client
pub struct PrometheusClient {
    pub endpoint: PromEndpoint,
}

impl PrometheusClient {
    pub fn from_endpoint(endpoint: PromEndpoint) -> Self {
        Self { endpoint }
    }

    /// Parse a PromQL query string and execute it
    #[allow(dead_code)] // planned API: raw PromQL passthrough for ad-hoc queries
    pub async fn execute_query(&self, query: &str) -> Result<serde_json::Value> {
        self.endpoint
            .get_json(&format!(
                "/api/v1/query?query={}",
                urlencoding::encode(query)
            ))
            .await
    }
}

#[async_trait::async_trait]
impl MetricsProviderTrait for PrometheusClient {
    async fn query_instant(&self, query: &str, time: Option<u64>) -> Result<InstantQueryResponse> {
        let mut path = format!("/api/v1/query?query={}", urlencoding::encode(query));
        if let Some(t) = time {
            path.push_str(&format!("&time={}", t));
        }

        let response = self.endpoint.get_json(&path).await?;

        // Extract the data section
        let data = response
            .get("data")
            .ok_or_else(|| anyhow::anyhow!("No data in Prometheus response"))?;

        let result_type = data
            .get("resultType")
            .and_then(|v| v.as_str())
            .unwrap_or("vector")
            .to_string();

        let result = if let Some(arr) = data.get("result").and_then(|v| v.as_array()) {
            arr.iter()
                .filter_map(|sample| {
                    let metric = sample
                        .get("metric")
                        .and_then(|m| m.as_object())
                        .map(|m| {
                            m.iter()
                                .map(|(k, v)| (k.clone(), v.as_str().unwrap_or("").to_string()))
                                .collect::<BTreeMap<_, _>>()
                        })
                        .unwrap_or_default();

                    let value = sample
                        .get("value")
                        .and_then(|v| v.as_array())
                        .and_then(|arr| {
                            let timestamp = arr.first().and_then(parse_sample_timestamp)?;
                            let val = arr.get(1).and_then(|v| v.as_str()).map(|s| s.to_string())?;
                            Some(MetricValue {
                                timestamp,
                                value: val,
                            })
                        })?;

                    Some(crate::protocol::MetricSample { metric, value })
                })
                .collect()
        } else {
            Vec::new()
        };

        Ok(InstantQueryResponse {
            result_type,
            result,
        })
    }

    async fn query_range(
        &self,
        query: &str,
        start: u64,
        end: u64,
        step: &str,
    ) -> Result<serde_json::Value> {
        let path = format!(
            "/api/v1/query_range?query={}&start={}&end={}&step={}",
            urlencoding::encode(query),
            start,
            end,
            urlencoding::encode(step)
        );
        self.endpoint.get_json(&path).await
    }

    async fn get_metadata(
        &self,
        metric_name: Option<&str>,
    ) -> Result<BTreeMap<String, Vec<MetricMetadata>>> {
        let path = if let Some(name) = metric_name {
            format!("/api/v1/metadata?metric={}", urlencoding::encode(name))
        } else {
            "/api/v1/metadata".to_owned()
        };

        let json = self.endpoint.get_json(&path).await?;

        let mut metadata = BTreeMap::new();

        if let Some(data) = json.get("data").and_then(|v| v.as_object()) {
            for (metric_name, metas) in data.iter() {
                let meta_list = if let Some(arr) = metas.as_array() {
                    arr.iter()
                        .map(|m| {
                            let metric_type = m
                                .get("type")
                                .and_then(|v| v.as_str())
                                .unwrap_or("unknown")
                                .to_string();
                            let help = m
                                .get("help")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string();
                            let unit = m
                                .get("unit")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string();
                            MetricMetadata {
                                metric_type,
                                help,
                                unit,
                            }
                        })
                        .collect()
                } else {
                    Vec::new()
                };
                metadata.insert(metric_name.clone(), meta_list);
            }
        }

        Ok(metadata)
    }
}

/// Helper function to query pod metrics from Prometheus
pub async fn query_pod_metrics(
    client: &PrometheusClient,
    namespace: &str,
    pod_name: &str,
) -> Result<PrometheusMetrics> {
    use crate::protocol::queries;

    let cpu_query = queries::POD_CPU_USAGE_PERCENT
        .replace("{pod}", pod_name)
        .replace("{namespace}", namespace);

    let memory_query = queries::POD_MEMORY_USAGE_BYTES
        .replace("{pod}", pod_name)
        .replace("{namespace}", namespace);

    let memory_limit_query = queries::POD_MEMORY_LIMIT_BYTES
        .replace("{pod}", pod_name)
        .replace("{namespace}", namespace);

    let rx_query = queries::POD_NETWORK_RX_BYTES
        .replace("{pod}", pod_name)
        .replace("{namespace}", namespace);

    let tx_query = queries::POD_NETWORK_TX_BYTES
        .replace("{pod}", pod_name)
        .replace("{namespace}", namespace);

    // Execute all queries in parallel
    let (cpu_result, memory_result, memory_limit_result, rx_result, tx_result) = tokio::join!(
        client.query_instant(&cpu_query, None),
        client.query_instant(&memory_query, None),
        client.query_instant(&memory_limit_query, None),
        client.query_instant(&rx_query, None),
        client.query_instant(&tx_query, None),
    );

    // Every query failing means the endpoint is unreachable, not that the pod has no data:
    // report it so the caller can fall back to the metrics API.
    if let (Err(e), Err(_), Err(_), Err(_), Err(_)) = (
        &cpu_result,
        &memory_result,
        &memory_limit_result,
        &rx_result,
        &tx_result,
    ) {
        anyhow::bail!("Prometheus queries failed: {e}");
    }

    // Extract values - clone to avoid lifetime issues
    let cpu_usage_percent = cpu_result
        .ok()
        .and_then(|r| r.result.first().cloned())
        .and_then(|s| s.value.value.parse::<f64>().ok().filter(|v| v.is_finite()));

    let memory_usage_bytes = memory_result
        .ok()
        .and_then(|r| r.result.first().cloned())
        .and_then(|s| parse_sample_u64(&s.value.value));

    let memory_limit_bytes = memory_limit_result
        .ok()
        .and_then(|r| r.result.first().cloned())
        .and_then(|s| parse_sample_u64(&s.value.value));

    let memory_usage_percent = match (memory_usage_bytes, memory_limit_bytes) {
        (Some(usage), Some(limit)) if limit > 0 => Some((usage as f64 / limit as f64) * 100.0),
        _ => None,
    };

    let rx_bytes = rx_result
        .ok()
        .and_then(|r| r.result.first().cloned())
        .and_then(|s| parse_sample_u64(&s.value.value));

    let tx_bytes = tx_result
        .ok()
        .and_then(|r| r.result.first().cloned())
        .and_then(|s| parse_sample_u64(&s.value.value));

    Ok(PrometheusMetrics {
        namespace: namespace.to_string(),
        name: pod_name.to_string(),
        cpu: CpuMetrics {
            usage_percent: cpu_usage_percent,
            cores: None,
        },
        memory: MemoryMetrics {
            usage_bytes: memory_usage_bytes,
            usage_percent: memory_usage_percent,
            limit_bytes: memory_limit_bytes,
        },
        network: NetworkMetrics {
            bytes_in: rx_bytes,
            bytes_out: tx_bytes,
            packets_in: None,
            packets_out: None,
        },
        timestamp: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn float_timestamps_parse() {
        assert_eq!(
            parse_sample_timestamp(&serde_json::json!(1790448285.378)),
            Some(1790448285)
        );
        assert_eq!(parse_sample_timestamp(&serde_json::json!(12)), Some(12));
        assert_eq!(parse_sample_timestamp(&serde_json::json!("x")), None);
    }

    #[test]
    fn sample_values_parse_floats_and_scientific() {
        assert_eq!(parse_sample_u64("12345"), Some(12345));
        assert_eq!(parse_sample_u64("1.2e+08"), Some(120_000_000));
        assert_eq!(parse_sample_u64("3.6"), Some(4));
        assert_eq!(parse_sample_u64("NaN"), None);
        assert_eq!(parse_sample_u64("+Inf"), None);
    }

    #[test]
    fn service_proxy_path_targets_named_port() {
        assert_eq!(
            service_proxy_path("mon", "prometheus", 9090, "/api/v1/query?query=up"),
            "/api/v1/namespaces/mon/services/prometheus:9090/proxy/api/v1/query?query=up"
        );
    }
}
