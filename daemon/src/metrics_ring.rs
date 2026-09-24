//! In-memory metrics ring (spec 036 R12, FR-019): ~240 samples per series (15 s => ~1 h),
//! total bytes bounded, explicit truncation marker. Also the metrics.k8s.io sampler.

use crate::hub::Hub;
use crate::series_store::{RangeResult, Sample, SeriesStore};
use chrono::{DateTime, Utc};
use fathom::protocol::v1::{MetricKind, SeriesKey};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

pub const DEFAULT_SAMPLES_PER_SERIES: usize = 240;
pub const DEFAULT_MAX_BYTES: usize = 32 * 1024 * 1024;
pub const SAMPLE_INTERVAL: Duration = Duration::from_secs(15);
const SAMPLE_BYTES: usize = 16;
const SERIES_OVERHEAD: usize = 128;

struct Series {
    samples: VecDeque<Sample>,
    dropped: bool,
}

struct Inner {
    series: HashMap<SeriesKey, Series>,
    bytes: usize,
}

pub struct MetricsRing {
    cap: usize,
    max_bytes: usize,
    inner: Mutex<Inner>,
}

impl MetricsRing {
    pub fn new(cap_per_series: usize, max_bytes: usize) -> Self {
        Self {
            cap: cap_per_series.max(1),
            max_bytes,
            inner: Mutex::new(Inner {
                series: HashMap::new(),
                bytes: 0,
            }),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub fn approx_bytes(&self) -> usize {
        self.lock().bytes
    }

    pub fn series_count(&self) -> usize {
        self.lock().series.len()
    }

    /// Drop series whose object no longer exists.
    pub fn prune(&self, live_uids: &HashSet<String>) {
        let mut g = self.lock();
        let dead: Vec<SeriesKey> = g
            .series
            .keys()
            .filter(|k| !live_uids.contains(&k.object_uid))
            .cloned()
            .collect();
        for k in dead {
            if let Some(s) = g.series.remove(&k) {
                g.bytes -= SERIES_OVERHEAD + s.samples.len() * SAMPLE_BYTES;
            }
        }
    }
}

impl SeriesStore for MetricsRing {
    fn append(&self, key: &SeriesKey, sample: Sample) -> bool {
        let mut g = self.lock();
        let max = self.max_bytes;
        let cap = self.cap;
        if !g.series.contains_key(key) {
            if g.bytes + SERIES_OVERHEAD + SAMPLE_BYTES > max {
                return false;
            }
            g.bytes += SERIES_OVERHEAD;
            g.series.insert(
                key.clone(),
                Series {
                    samples: VecDeque::new(),
                    dropped: false,
                },
            );
        }
        let full = g.series[key].samples.len() >= cap;
        if !full && g.bytes + SAMPLE_BYTES > max {
            return false;
        }
        let grow = if full { 0 } else { SAMPLE_BYTES };
        let s = g.series.get_mut(key).expect("inserted above");
        if full {
            s.samples.pop_front();
            s.dropped = true;
        }
        s.samples.push_back(sample);
        g.bytes += grow;
        true
    }

    fn query_range(&self, key: &SeriesKey, from: DateTime<Utc>, to: DateTime<Utc>) -> RangeResult {
        let g = self.lock();
        let Some(s) = g.series.get(key) else {
            return RangeResult::default();
        };
        let oldest = s.samples.front().map(|x| x.time);
        RangeResult {
            samples: s
                .samples
                .iter()
                .filter(|x| x.time >= from && x.time <= to)
                .copied()
                .collect(),
            truncated_oldest: oldest.filter(|o| s.dropped && from < *o),
        }
    }

    fn truncate_marker(&self, key: &SeriesKey) -> Option<DateTime<Utc>> {
        let g = self.lock();
        let s = g.series.get(key)?;
        s.dropped
            .then(|| s.samples.front().map(|x| x.time))
            .flatten()
    }
}

/// CPU quantity -> cores (`250m`, `1500000n`, `2`).
pub fn parse_cpu_cores(q: &str) -> Option<f64> {
    let (num, div) = if let Some(n) = q.strip_suffix('n') {
        (n, 1e9)
    } else if let Some(n) = q.strip_suffix('u') {
        (n, 1e6)
    } else if let Some(n) = q.strip_suffix('m') {
        (n, 1e3)
    } else {
        (q, 1.0)
    };
    num.parse::<f64>().ok().map(|v| v / div)
}

/// Memory quantity -> bytes (`128Ki`, `2Mi`, `1G`, plain integer).
pub fn parse_memory_bytes(q: &str) -> Option<f64> {
    const UNITS: &[(&str, f64)] = &[
        ("Ki", 1024.0),
        ("Mi", 1024.0 * 1024.0),
        ("Gi", 1024.0 * 1024.0 * 1024.0),
        ("Ti", 1024.0 * 1024.0 * 1024.0 * 1024.0),
        ("k", 1e3),
        ("M", 1e6),
        ("G", 1e9),
        ("T", 1e12),
    ];
    for (suf, mul) in UNITS {
        if let Some(n) = q.strip_suffix(suf) {
            return n.parse::<f64>().ok().map(|v| v * mul);
        }
    }
    q.parse::<f64>().ok()
}

#[derive(Debug, Clone, PartialEq)]
pub struct Reading {
    pub namespace: Option<String>,
    pub name: String,
    pub cpu: f64,
    pub memory: f64,
}

/// Parse a `metrics.k8s.io` PodMetricsList (`pods = true`) or NodeMetricsList.
pub fn readings_from_list(list: &serde_json::Value, pods: bool) -> Vec<Reading> {
    let usage = |u: &serde_json::Value| -> (f64, f64) {
        (
            u["cpu"].as_str().and_then(parse_cpu_cores).unwrap_or(0.0),
            u["memory"]
                .as_str()
                .and_then(parse_memory_bytes)
                .unwrap_or(0.0),
        )
    };
    let Some(items) = list["items"].as_array() else {
        return vec![];
    };
    items
        .iter()
        .filter_map(|i| {
            let name = i["metadata"]["name"].as_str()?.to_string();
            let (cpu, memory) = if pods {
                i["containers"]
                    .as_array()?
                    .iter()
                    .map(|c| usage(&c["usage"]))
                    .fold((0.0, 0.0), |a, b| (a.0 + b.0, a.1 + b.1))
            } else {
                usage(&i["usage"])
            };
            Some(Reading {
                namespace: pods
                    .then(|| i["metadata"]["namespace"].as_str().map(str::to_string))
                    .flatten(),
                name,
                cpu,
                memory,
            })
        })
        .collect()
}

/// Whether the metrics API is served; shared with the HTTP layer for `/v1/capabilities`.
#[derive(Default)]
pub struct MetricsAvailability(AtomicBool);

impl MetricsAvailability {
    pub fn set(&self, v: bool) {
        self.0.store(v, Ordering::Relaxed);
    }
    pub fn get(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

async fn fetch(client: &kube::Client, path: &str) -> anyhow::Result<Option<serde_json::Value>> {
    let req = http_get(path)?;
    match client.request::<serde_json::Value>(req).await {
        Ok(v) => Ok(Some(v)),
        Err(kube::Error::Api(e)) if e.code == 404 => Ok(None),
        Err(e) => Err(e.into()),
    }
}

fn http_get(path: &str) -> anyhow::Result<axum::http::Request<Vec<u8>>> {
    Ok(axum::http::Request::get(path).body(Vec::new())?)
}

fn record(
    store: &dyn SeriesStore,
    hub: &Hub,
    readings: Vec<Reading>,
    kind: &str,
    now: DateTime<Utc>,
    live: &mut HashSet<String>,
) {
    for r in readings {
        let Some(uid) = hub.uid_of(kind, r.namespace.as_deref(), &r.name) else {
            continue;
        };
        for (metric, value) in [(MetricKind::Cpu, r.cpu), (MetricKind::Memory, r.memory)] {
            store.append(
                &SeriesKey {
                    object_uid: uid.clone(),
                    metric,
                },
                Sample { time: now, value },
            );
        }
        live.insert(uid);
    }
}

/// Sample metrics.k8s.io every 15 s while the API exists. Missing API flips `avail` to false
/// (reported as `metricsUnavailable`); it is retried so a later-installed metrics-server works.
pub async fn run_sampler(
    client: kube::Client,
    hub: std::sync::Arc<Hub>,
    ring: std::sync::Arc<MetricsRing>,
    avail: std::sync::Arc<MetricsAvailability>,
) {
    let mut tick = tokio::time::interval(SAMPLE_INTERVAL);
    loop {
        tick.tick().await;
        let now = Utc::now();
        let mut live = HashSet::new();
        let pods = fetch(&client, "/apis/metrics.k8s.io/v1beta1/pods").await;
        let nodes = fetch(&client, "/apis/metrics.k8s.io/v1beta1/nodes").await;
        let mut any = false;
        for (res, is_pods, kind) in [(pods, true, "Pod"), (nodes, false, "Node")] {
            match res {
                Ok(Some(v)) => {
                    any = true;
                    record(
                        ring.as_ref(),
                        &hub,
                        readings_from_list(&v, is_pods),
                        kind,
                        now,
                        &mut live,
                    );
                }
                Ok(None) => {}
                Err(e) => tracing::warn!(error = %e, "metrics sample failed"),
            }
        }
        avail.set(any);
        if any {
            // Objects absent from this round (deleted or no longer reported) lose their series.
            ring.prune(&live);
        }
    }
}
