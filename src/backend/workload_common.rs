use crate::protocol::PodSummary;
use chrono::{DateTime, Utc};
use k8s_openapi::api::core::v1::Pod;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::{MicroTime, Time};
use k8s_openapi::jiff::Timestamp;
use kube::{
    ResourceExt,
    api::{Api, DeleteParams, ListParams},
};
use std::collections::BTreeMap;

use super::pods;
use super::state::BackendState;

pub(super) fn map_opt_btreemap(map: Option<BTreeMap<String, String>>) -> BTreeMap<String, String> {
    map.unwrap_or_default().into_iter().collect()
}

pub(super) fn creation_timestamp(meta: &kube::core::ObjectMeta) -> String {
    meta.creation_timestamp
        .as_ref()
        .map(kube_rfc3339)
        .unwrap_or_default()
}

/// The two Kubernetes timestamp types, both jiff `Timestamp`s since k8s-openapi 0.28.
pub(super) trait KubeStamp {
    fn stamp(&self) -> Timestamp;
}

impl KubeStamp for Time {
    fn stamp(&self) -> Timestamp {
        self.0
    }
}

impl KubeStamp for MicroTime {
    fn stamp(&self) -> Timestamp {
        self.0
    }
}

impl<T: KubeStamp> KubeStamp for &T {
    fn stamp(&self) -> Timestamp {
        (**self).stamp()
    }
}

/// A Kubernetes `Time` (or `MicroTime`) as chrono, which the rest of the backend speaks. k8s-openapi 0.28 moved
/// `Time` onto jiff; converting at this seam keeps every timestamp the UI sees in chrono's RFC 3339
/// form (`+00:00`, not jiff's `Z`).
pub(super) fn kube_time(t: &impl KubeStamp) -> DateTime<Utc> {
    let stamp = t.stamp();
    // jiff's sub-second part carries the sign of the whole timestamp; chrono's is always positive.
    let (mut secs, mut nanos) = (stamp.as_second(), stamp.subsec_nanosecond());
    if nanos < 0 {
        secs -= 1;
        nanos += 1_000_000_000;
    }
    DateTime::from_timestamp(secs, nanos.unsigned_abs()).unwrap_or(DateTime::<Utc>::MIN_UTC)
}

/// [`kube_time`], formatted as the backend has always sent it.
pub(super) fn kube_rfc3339(t: &impl KubeStamp) -> String {
    kube_time(t).to_rfc3339()
}

/// The inverse of [`kube_time`], for building fixtures from chrono time.
#[cfg(test)]
pub(super) fn to_kube_time(t: DateTime<Utc>) -> Time {
    let nanos = t.timestamp_subsec_nanos();
    let stamp = Timestamp::new(t.timestamp(), 0)
        .and_then(|s| {
            s.checked_add(k8s_openapi::jiff::SignedDuration::from_nanos(i64::from(
                nanos,
            )))
        })
        .unwrap_or(Timestamp::MIN);
    Time(stamp)
}

pub(super) fn containers_from_pod_template(
    template: Option<&k8s_openapi::api::core::v1::PodTemplateSpec>,
) -> Vec<String> {
    template
        .and_then(|t| t.spec.as_ref())
        .map(|ps| {
            ps.containers
                .iter()
                .map(|c| format!("{} ({})", c.name, c.image.clone().unwrap_or_default()))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

pub(super) fn ports_from_pod_template(
    template: Option<&k8s_openapi::api::core::v1::PodTemplateSpec>,
) -> Vec<crate::protocol::ContainerPort> {
    template
        .and_then(|t| t.spec.as_ref())
        .map(|ps| {
            ps.containers
                .iter()
                .flat_map(|c| {
                    c.ports
                        .iter()
                        .flatten()
                        .map(|p| crate::protocol::ContainerPort {
                            container: c.name.clone(),
                            name: p.name.clone(),
                            port: p.container_port as u16,
                            protocol: p.protocol.clone().unwrap_or_else(|| "TCP".to_owned()),
                        })
                })
                .collect()
        })
        .unwrap_or_default()
}

pub(super) fn selector_string_from_match_labels(
    match_labels: Option<&BTreeMap<String, String>>,
) -> String {
    match_labels
        .map(|labels| {
            let mut parts: Vec<String> =
                labels.iter().map(|(k, v)| format!("{}={}", k, v)).collect();
            parts.sort();
            parts.join(",")
        })
        .unwrap_or_default()
}

pub(super) fn selector_string_from_map(map: Option<&BTreeMap<String, String>>) -> String {
    selector_string_from_match_labels(map)
}

pub(super) async fn pods_for_selector(
    state: &BackendState,
    namespace: &str,
    selector: &str,
) -> anyhow::Result<Vec<PodSummary>> {
    if selector.is_empty() {
        return Ok(Vec::new());
    }

    let client = state.cluster.client_or_err()?;
    let pod_api: Api<Pod> = Api::namespaced(client, namespace);
    let list = pod_api
        .list(&ListParams::default().labels(selector))
        .await?;
    Ok(pods::pods_to_summaries(list.items, &[]))
}

pub(super) async fn delete_pods_for_selector(
    state: &BackendState,
    namespace: &str,
    selector: &str,
) -> anyhow::Result<u32> {
    if selector.is_empty() {
        return Ok(0);
    }

    let client = state.cluster.client_or_err()?;
    let pod_api: Api<Pod> = Api::namespaced(client, namespace);
    let list = pod_api
        .list(&ListParams::default().labels(selector))
        .await?;

    let mut deleted: u32 = 0;
    for pod in list.items {
        let name = pod.name_any();
        // Ignore per-pod delete errors to be best-effort.
        if pod_api
            .delete(&name, &DeleteParams::default())
            .await
            .is_ok()
        {
            deleted += 1;
        }
    }

    Ok(deleted)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kube_time_keeps_the_chrono_rfc3339_form() {
        let t: Time = Time("2026-10-22T19:16:58.250Z".parse().unwrap());
        assert_eq!(kube_rfc3339(&t), "2026-10-22T19:16:58.250+00:00");
        let whole: Time = Time("2026-10-22T19:16:58Z".parse().unwrap());
        assert_eq!(kube_rfc3339(&whole), "2026-10-22T19:16:58+00:00");
    }

    #[test]
    fn kube_time_round_trips_including_before_the_epoch() {
        for s in [
            "2026-10-22T19:16:58.123456789Z",
            "1969-12-31T23:59:59.5Z",
            "1970-01-01T00:00:00Z",
        ] {
            let t = Time(s.parse().unwrap());
            let c = kube_time(&t);
            assert_eq!(to_kube_time(c), t, "{s}");
            assert_eq!(
                c,
                DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc),
                "{s}"
            );
        }
    }
}
