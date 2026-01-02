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
    async fn query_range(
        &self,
        query: &str,
        start: u64,
        end: u64,
        step: &str,
    ) -> Result<serde_json::Value>;

    /// Get available metrics metadata
    async fn get_metadata(
        &self,
        metric_name: Option<&str>,
    ) -> Result<BTreeMap<String, Vec<MetricMetadata>>>;
}

#[derive(Debug, Clone)]
pub struct MetricMetadata {
    pub metric_type: String,
    pub help: String,
    pub unit: String,
}

/// Prometheus HTTP API client
pub struct PrometheusClient {
    pub url: String,
    pub client: reqwest::Client,
}

impl PrometheusClient {
    pub fn new(url: String) -> Self {
        Self {
            url,
            client: reqwest::Client::new(),
        }
    }

    /// Parse a PromQL query string and execute it
    pub async fn execute_query(&self, query: &str) -> Result<serde_json::Value> {
        let encoded_query = urlencoding::encode(query);
        let url = format!("{}/api/v1/query?query={}", self.url, encoded_query);

        let response = self.client.get(url).send().await?;

        Ok(response.json().await?)
    }
}

#[async_trait::async_trait]
impl MetricsProviderTrait for PrometheusClient {
    async fn query_instant(&self, query: &str, time: Option<u64>) -> Result<InstantQueryResponse> {
        let mut url = format!(
            "{}/api/v1/query?query={}",
            self.url,
            urlencoding::encode(query)
        );
        if let Some(t) = time {
            url.push_str(&format!("&time={}", t));
        }

        let response = self
            .client
            .get(url)
            .send()
            .await?
            .json::<serde_json::Value>()
            .await?;

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
                            let timestamp = arr.first().and_then(|v| v.as_u64())?;
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
        let url = format!(
            "{}/api/v1/query_range?query={}&start={}&end={}&step={}",
            self.url,
            urlencoding::encode(query),
            start,
            end,
            urlencoding::encode(step)
        );

        let response = self
            .client
            .get(url)
            .send()
            .await?
            .json::<serde_json::Value>()
            .await?;

        Ok(response)
    }

    async fn get_metadata(
        &self,
        metric_name: Option<&str>,
    ) -> Result<BTreeMap<String, Vec<MetricMetadata>>> {
        let url = if let Some(name) = metric_name {
            format!(
                "{}/api/v1/metadata?metric={}",
                self.url,
                urlencoding::encode(name)
            )
        } else {
            format!("{}/api/v1/metadata", self.url)
        };

        let response = self.client.get(url).send().await?;

        let json = response.json::<serde_json::Value>().await?;

        let mut metadata = BTreeMap::new();

        if let Some(data) = json.get("data").and_then(|v| v.as_object()) {
            for (metric_name, metas) in data.iter() {
                let meta_list = if let Some(arr) = metas.as_array() {
                    arr.iter()
                        .filter_map(|m| {
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
                            Some(MetricMetadata {
                                metric_type,
                                help,
                                unit,
                            })
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

    // Extract values - clone to avoid lifetime issues
    let cpu_usage_percent = cpu_result
        .ok()
        .and_then(|r| r.result.first().cloned())
        .map(|s| s.value.value.parse::<f64>().unwrap_or(0.0));

    let memory_usage_bytes = memory_result
        .ok()
        .and_then(|r| r.result.first().cloned())
        .map(|s| s.value.value.parse::<u64>().unwrap_or(0));

    let memory_limit_bytes = memory_limit_result
        .ok()
        .and_then(|r| r.result.first().cloned())
        .map(|s| s.value.value.parse::<u64>().unwrap_or(0));

    let memory_usage_percent = match (memory_usage_bytes, memory_limit_bytes) {
        (Some(usage), Some(limit)) if limit > 0 => Some((usage as f64 / limit as f64) * 100.0),
        _ => None,
    };

    let rx_bytes = rx_result
        .ok()
        .and_then(|r| r.result.first().cloned())
        .map(|s| s.value.value.parse::<u64>().unwrap_or(0));

    let tx_bytes = tx_result
        .ok()
        .and_then(|r| r.result.first().cloned())
        .map(|s| s.value.value.parse::<u64>().unwrap_or(0));

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
