//! KEDA autoscaler integration (029-keda-autoscaler-pane).
//!
//! Discovers the served `keda.sh` CRDs (`scaledobjects.keda.sh`, `scaledjobs.keda.sh`,
//! `triggerauthentications.keda.sh`, `clustertriggerauthentications.keda.sh`) and reads
//! ScaledObjects/ScaledJobs as dynamic objects - the same version-agnostic discovery pattern as
//! `flux.rs`. Strictly read-only: never mutates KEDA objects and never resolves the secrets that
//! a TriggerAuthentication references (only its name is surfaced).
//!
//! KEDA defaults used when a spec field is unset: `minReplicaCount: 0`, `maxReplicaCount: 100`.

use crate::backend::metrics_provider::MetricsProviderTrait;
use crate::protocol::{
    KedaOverview, KedaResourceDetails, KedaResourceSummary, KedaTrigger, ScaleReasoning,
    ScaleReasoningState, TriggerAuthRef,
};
use k8s_openapi::api::apps::v1::{Deployment, StatefulSet};
use k8s_openapi::api::batch::v1::Job;
use k8s_openapi::apiextensions_apiserver::pkg::apis::apiextensions::v1::CustomResourceDefinition;
use kube::api::ListParams;
use kube::core::DynamicObject;
use kube::core::{ApiResource, GroupVersionKind};
use kube::{Client, api::Api};
use serde_json::Value;
use std::collections::BTreeMap;

pub(crate) const SCALEDOBJECT_CRD: &str = "scaledobjects.keda.sh";
pub(crate) const SCALEDJOB_CRD: &str = "scaledjobs.keda.sh";
#[allow(dead_code)] // documented for completeness; auth refs are surfaced by name only
pub(crate) const TRIGGERAUTH_CRD: &str = "triggerauthentications.keda.sh";
#[allow(dead_code)]
pub(crate) const CLUSTERTRIGGERAUTH_CRD: &str = "clustertriggerauthentications.keda.sh";

const DEFAULT_MIN_REPLICAS: i32 = 0;
const DEFAULT_MAX_REPLICAS: i32 = 100;
const PAUSED_ANNOTATION: &str = "autoscaling.keda.sh/paused";

#[derive(Debug, Clone)]
pub(crate) struct DiscoveredCrd {
    pub group: String,
    pub version: String,
    pub kind: String,
}

/// Find a served version for `crd_name` among already-listed CRDs.
pub(crate) fn find_served(
    crds: &[CustomResourceDefinition],
    crd_name: &str,
) -> Option<DiscoveredCrd> {
    let crd = crds
        .iter()
        .find(|c| c.metadata.name.as_deref() == Some(crd_name))?;
    let version = crd
        .spec
        .versions
        .iter()
        .find(|v| v.served)
        .map(|v| v.name.clone())?;
    Some(DiscoveredCrd {
        group: crd.spec.group.clone(),
        version,
        kind: crd.spec.names.kind.clone(),
    })
}

fn api_for(client: &Client, crd: &DiscoveredCrd, namespace: Option<&str>) -> Api<DynamicObject> {
    let gvk = GroupVersionKind::gvk(&crd.group, &crd.version, &crd.kind);
    let resource = ApiResource::from_gvk(&gvk);
    match namespace {
        Some(ns) => Api::namespaced_with(client.clone(), ns, &resource),
        None => Api::all_with(client.clone(), &resource),
    }
}

async fn list_instances(client: &Client, crd: &DiscoveredCrd) -> Vec<DynamicObject> {
    api_for(client, crd, None)
        .list(&ListParams::default())
        .await
        .map(|l| l.items)
        .unwrap_or_default()
}

/// Live facts about a scale target, gathered by the caller (kept separate so the summary
/// derivation is pure and unit-testable).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TargetFacts {
    pub found: bool,
    pub replicas: Option<i32>,
}

fn as_i32(v: Option<&Value>) -> Option<i32> {
    v.and_then(|v| v.as_i64()).map(|n| n as i32)
}

/// `(kind, name)` of the scale target. ScaledObject defaults to `Deployment`; a ScaledJob
/// always targets Jobs it creates itself, so it reports its own name.
pub(crate) fn target_ref(obj: &DynamicObject, kind: &str) -> (String, String) {
    if kind == "ScaledJob" {
        return (
            "Job".to_owned(),
            obj.metadata.name.clone().unwrap_or_default(),
        );
    }
    let r = obj.data.get("spec").and_then(|s| s.get("scaleTargetRef"));
    let tk = r
        .and_then(|r| r.get("kind"))
        .and_then(|v| v.as_str())
        .unwrap_or("Deployment")
        .to_owned();
    let tn = r
        .and_then(|r| r.get("name"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_owned();
    (tk, tn)
}

fn ready_from_conditions(status: Option<&Value>) -> bool {
    status
        .and_then(|s| s.get("conditions"))
        .and_then(|c| c.as_array())
        .map(|cs| {
            cs.iter().any(|c| {
                matches!(
                    c.get("type").and_then(|t| t.as_str()),
                    Some("Ready") | Some("Active")
                ) && c.get("status").and_then(|s| s.as_str()) == Some("True")
            })
        })
        .unwrap_or(false)
}

/// Build a summary row from a ScaledObject/ScaledJob dynamic object plus live target facts.
pub(crate) fn summarize(
    obj: &DynamicObject,
    kind: &str,
    target: TargetFacts,
) -> KedaResourceSummary {
    let spec = obj.data.get("spec");
    let status = obj.data.get("status");
    let (target_kind, target_name) = target_ref(obj, kind);

    let min_replicas =
        as_i32(spec.and_then(|s| s.get("minReplicaCount"))).unwrap_or(DEFAULT_MIN_REPLICAS);
    let max_replicas =
        as_i32(spec.and_then(|s| s.get("maxReplicaCount"))).unwrap_or(DEFAULT_MAX_REPLICAS);
    let current_replicas = as_i32(status.and_then(|s| s.get("hpaReplicas")))
        .or(target.replicas)
        .unwrap_or(0);
    let paused = obj
        .metadata
        .annotations
        .as_ref()
        .and_then(|a| a.get(PAUSED_ANNOTATION))
        .map(|v| v != "false")
        .unwrap_or(false);

    let mut trigger_types: Vec<String> = Vec::new();
    for t in spec
        .and_then(|s| s.get("triggers"))
        .and_then(|t| t.as_array())
        .into_iter()
        .flatten()
    {
        if let Some(ty) = t.get("type").and_then(|v| v.as_str())
            && !trigger_types.iter().any(|x| x == ty)
        {
            trigger_types.push(ty.to_owned());
        }
    }

    KedaResourceSummary {
        kind: kind.to_owned(),
        name: obj.metadata.name.clone().unwrap_or_default(),
        namespace: obj.metadata.namespace.clone().unwrap_or_default(),
        target_kind,
        target_name,
        target_found: target.found,
        current_replicas,
        min_replicas,
        max_replicas,
        paused,
        ready: ready_from_conditions(status),
        trigger_types,
        age: super::workload_common::creation_timestamp(&obj.metadata),
    }
}

/// Derive the plain-language scale reasoning, rules in strict precedence order
/// (data-model.md "Derivation rules").
pub(crate) fn derive_scale_reasoning(s: &KedaResourceSummary) -> ScaleReasoning {
    let (state, message) = if !s.target_found {
        (
            ScaleReasoningState::TargetNotFound,
            "Target workload no longer exists.".to_owned(),
        )
    } else if s.paused {
        (
            ScaleReasoningState::Paused,
            "Paused via the autoscaling.keda.sh/paused annotation.".to_owned(),
        )
    } else if s.min_replicas == 0 && s.current_replicas == 0 {
        (
            ScaleReasoningState::ScaledToZero,
            "Scaled to zero - idle, no active demand.".to_owned(),
        )
    } else if s.current_replicas <= s.min_replicas {
        (
            ScaleReasoningState::AtFloor,
            format!(
                "At floor ({}) - no trigger currently reports active demand.",
                s.min_replicas
            ),
        )
    } else if s.current_replicas >= s.max_replicas {
        (
            ScaleReasoningState::AtCeiling,
            format!(
                "At ceiling ({}) - demand may exceed what the configured maximum allows.",
                s.max_replicas
            ),
        )
    } else {
        (
            ScaleReasoningState::MidRange,
            format!(
                "Scaled to {} between the minimum ({}) and maximum ({}) as trigger demand changes.",
                s.current_replicas, s.min_replicas, s.max_replicas
            ),
        )
    };
    ScaleReasoning { state, message }
}

/// Parse `spec.triggers[]` verbatim (no secret resolution, no live values).
pub(crate) fn extract_triggers(obj: &DynamicObject) -> Vec<KedaTrigger> {
    obj.data
        .get("spec")
        .and_then(|s| s.get("triggers"))
        .and_then(|t| t.as_array())
        .into_iter()
        .flatten()
        .map(|t| {
            let metadata: BTreeMap<String, String> = t
                .get("metadata")
                .and_then(|m| m.as_object())
                .map(|m| {
                    m.iter()
                        .map(|(k, v)| {
                            (
                                k.clone(),
                                v.as_str()
                                    .map(str::to_owned)
                                    .unwrap_or_else(|| v.to_string()),
                            )
                        })
                        .collect()
                })
                .unwrap_or_default();
            let auth_ref = t.get("authenticationRef").and_then(|a| {
                let name = a.get("name").and_then(|n| n.as_str())?.to_owned();
                let cluster_scoped =
                    a.get("kind").and_then(|k| k.as_str()) == Some("ClusterTriggerAuthentication");
                Some(TriggerAuthRef {
                    name,
                    cluster_scoped,
                })
            });
            let threshold = metadata
                .get("threshold")
                .or_else(|| metadata.get("value"))
                .cloned();
            KedaTrigger {
                trigger_type: t
                    .get("type")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_owned(),
                metadata,
                auth_ref,
                active: None,
                current_value: None,
                threshold,
            }
        })
        .collect()
}

/// Populate `current_value` for `prometheus` triggers using Fathom's own metrics provider.
/// Any failure leaves `current_value: None`; nothing propagates (FR-012/FR-013).
pub(crate) async fn enrich_prometheus_values(
    triggers: &mut [KedaTrigger],
    provider: Option<&(dyn MetricsProviderTrait + Sync)>,
) {
    let Some(provider) = provider else { return };
    for t in triggers
        .iter_mut()
        .filter(|t| t.trigger_type == "prometheus")
    {
        let Some(query) = t.metadata.get("query").cloned() else {
            continue;
        };
        if let Ok(resp) = provider.query_instant(&query, None).await
            && let Some(sample) = resp.result.first()
        {
            t.current_value = sample.value.value.parse::<f64>().ok();
        }
    }
}

async fn target_facts(
    client: &Client,
    namespace: &str,
    kind: &str,
    target_kind: &str,
    target_name: &str,
    scaled_job_name: &str,
) -> TargetFacts {
    if kind == "ScaledJob" {
        let api: Api<Job> = Api::namespaced(client.clone(), namespace);
        let active = api
            .list(
                &ListParams::default().labels(&format!("scaledjob.keda.sh/name={scaled_job_name}")),
            )
            .await
            .map(|l| {
                l.items
                    .iter()
                    .map(|j| j.status.as_ref().and_then(|s| s.active).unwrap_or(0))
                    .sum::<i32>()
            })
            .ok();
        return TargetFacts {
            found: true,
            replicas: active,
        };
    }
    match target_kind {
        "Deployment" => {
            let api: Api<Deployment> = Api::namespaced(client.clone(), namespace);
            match api.get_opt(target_name).await {
                Ok(Some(d)) => TargetFacts {
                    found: true,
                    replicas: d.status.and_then(|s| s.replicas),
                },
                Ok(None) => TargetFacts {
                    found: false,
                    replicas: None,
                },
                Err(_) => TargetFacts {
                    found: true,
                    replicas: None,
                },
            }
        }
        "StatefulSet" => {
            let api: Api<StatefulSet> = Api::namespaced(client.clone(), namespace);
            match api.get_opt(target_name).await {
                Ok(Some(d)) => TargetFacts {
                    found: true,
                    replicas: d.status.map(|s| s.replicas),
                },
                Ok(None) => TargetFacts {
                    found: false,
                    replicas: None,
                },
                Err(_) => TargetFacts {
                    found: true,
                    replicas: None,
                },
            }
        }
        // Custom scale targets: we cannot cheaply verify them, so do not claim they are missing.
        _ => TargetFacts {
            found: true,
            replicas: None,
        },
    }
}

async fn summarize_object(client: &Client, obj: &DynamicObject, kind: &str) -> KedaResourceSummary {
    let ns = obj.metadata.namespace.clone().unwrap_or_default();
    let (tk, tn) = target_ref(obj, kind);
    let name = obj.metadata.name.clone().unwrap_or_default();
    let facts = target_facts(client, &ns, kind, &tk, &tn, &name).await;
    summarize(obj, kind, facts)
}

/// Discover KEDA and build the full overview. `installed: false` only when neither the
/// ScaledObject nor ScaledJob CRD is served.
pub(super) async fn scan_keda(client: &Client) -> anyhow::Result<KedaOverview> {
    let crds = Api::<CustomResourceDefinition>::all(client.clone())
        .list(&ListParams::default())
        .await?
        .items;
    let so_crd = find_served(&crds, SCALEDOBJECT_CRD);
    let sj_crd = find_served(&crds, SCALEDJOB_CRD);
    let installed = so_crd.is_some() || sj_crd.is_some();

    let mut scaled_objects = Vec::new();
    if let Some(crd) = &so_crd {
        for obj in list_instances(client, crd).await {
            scaled_objects.push(summarize_object(client, &obj, "ScaledObject").await);
        }
    }
    let mut scaled_jobs = Vec::new();
    if let Some(crd) = &sj_crd {
        for obj in list_instances(client, crd).await {
            scaled_jobs.push(summarize_object(client, &obj, "ScaledJob").await);
        }
    }
    let key = |s: &KedaResourceSummary| (s.namespace.clone(), s.name.clone());
    scaled_objects.sort_by_key(key);
    scaled_jobs.sort_by_key(key);
    Ok(KedaOverview {
        installed,
        scaled_objects,
        scaled_jobs,
    })
}

/// Fetch one ScaledObject/ScaledJob for the Inspector. `Ok(None)` when it no longer exists.
pub(super) async fn get_details(
    client: &Client,
    kind: &str,
    namespace: &str,
    name: &str,
    provider: Option<&(dyn MetricsProviderTrait + Sync)>,
) -> anyhow::Result<Option<KedaResourceDetails>> {
    let crds = Api::<CustomResourceDefinition>::all(client.clone())
        .list(&ListParams::default())
        .await?
        .items;
    let crd_name = if kind == "ScaledJob" {
        SCALEDJOB_CRD
    } else {
        SCALEDOBJECT_CRD
    };
    let Some(crd) = find_served(&crds, crd_name) else {
        return Ok(None);
    };
    let Some(obj) = api_for(client, &crd, Some(namespace)).get_opt(name).await? else {
        return Ok(None);
    };
    let summary = summarize_object(client, &obj, kind).await;
    let mut triggers = extract_triggers(&obj);
    enrich_prometheus_values(&mut triggers, provider).await;
    let reasoning = derive_scale_reasoning(&summary);
    Ok(Some(KedaResourceDetails {
        summary,
        triggers,
        reasoning,
    }))
}

/// Resolve the `(kind, name)` a ScaledObject/ScaledJob in `namespace` scales, for `related.rs`.
pub(crate) async fn list_scale_targets(
    client: &Client,
    namespace: &str,
) -> Vec<(String, String, String, String)> {
    // (scaler kind, scaler name, target kind, target name)
    let Ok(crds) = Api::<CustomResourceDefinition>::all(client.clone())
        .list(&ListParams::default())
        .await
    else {
        return Vec::new();
    };
    let crds = crds.items;
    let mut out = Vec::new();
    if let Some(crd) = find_served(&crds, SCALEDOBJECT_CRD)
        && let Ok(list) = api_for(client, &crd, Some(namespace))
            .list(&ListParams::default())
            .await
    {
        for obj in list.items {
            let (tk, tn) = target_ref(&obj, "ScaledObject");
            out.push((
                "ScaledObject".to_owned(),
                obj.metadata.name.clone().unwrap_or_default(),
                tk,
                tn,
            ));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{InstantQueryResponse, MetricSample, MetricValue};
    use k8s_openapi::apiextensions_apiserver::pkg::apis::apiextensions::v1::{
        CustomResourceDefinitionNames, CustomResourceDefinitionSpec,
        CustomResourceDefinitionVersion,
    };
    use serde_json::json;

    fn crd(name: &str, served: bool) -> CustomResourceDefinition {
        let mut c = CustomResourceDefinition::default();
        c.metadata.name = Some(name.to_owned());
        c.spec = CustomResourceDefinitionSpec {
            group: "keda.sh".to_owned(),
            names: CustomResourceDefinitionNames {
                kind: "ScaledObject".to_owned(),
                plural: "scaledobjects".to_owned(),
                ..Default::default()
            },
            versions: vec![CustomResourceDefinitionVersion {
                name: "v1alpha1".to_owned(),
                served,
                storage: true,
                ..Default::default()
            }],
            scope: "Namespaced".to_owned(),
            ..Default::default()
        };
        c
    }

    fn obj(spec: Value, status: Value, annotations: Option<(&str, &str)>) -> DynamicObject {
        let mut o: DynamicObject = serde_json::from_value(json!({
            "apiVersion": "keda.sh/v1alpha1",
            "kind": "ScaledObject",
            "metadata": {"name": "queue-worker", "namespace": "default"},
            "spec": spec,
            "status": status,
        }))
        .unwrap();
        if let Some((k, v)) = annotations {
            o.metadata.annotations = Some([(k.to_owned(), v.to_owned())].into());
        }
        o
    }

    const FOUND: TargetFacts = TargetFacts {
        found: true,
        replicas: Some(2),
    };

    #[test]
    fn crd_detection_installed_and_absent() {
        let crds = vec![crd("scaledobjects.keda.sh", true)];
        assert!(find_served(&crds, SCALEDOBJECT_CRD).is_some());
        assert!(find_served(&crds, SCALEDJOB_CRD).is_none());
        let none = vec![crd("something.else.io", true)];
        assert!(find_served(&none, SCALEDOBJECT_CRD).is_none());
        assert!(find_served(&none, SCALEDJOB_CRD).is_none());
        // Present but no served version is not installed.
        assert!(find_served(&[crd("scaledobjects.keda.sh", false)], SCALEDOBJECT_CRD).is_none());
    }

    #[test]
    fn summary_defaults_min_max() {
        let o = obj(json!({"scaleTargetRef": {"name": "w"}}), json!({}), None);
        let s = summarize(&o, "ScaledObject", FOUND);
        assert_eq!(s.min_replicas, 0);
        assert_eq!(s.max_replicas, 100);
        assert_eq!(s.target_kind, "Deployment");
        assert_eq!(s.target_name, "w");
        assert_eq!(s.current_replicas, 2); // falls back to the live target
    }

    #[test]
    fn summary_reads_bounds_paused_and_hpa_replicas() {
        let o = obj(
            json!({"scaleTargetRef": {"name": "w", "kind": "StatefulSet"},
                   "minReplicaCount": 1, "maxReplicaCount": 10,
                   "triggers": [{"type": "cron"}, {"type": "cron"}, {"type": "kafka"}]}),
            json!({"hpaReplicas": 4, "conditions": [{"type": "Ready", "status": "True"}]}),
            Some((PAUSED_ANNOTATION, "true")),
        );
        let s = summarize(&o, "ScaledObject", FOUND);
        assert_eq!(
            (s.min_replicas, s.max_replicas, s.current_replicas),
            (1, 10, 4)
        );
        assert!(s.paused && s.ready);
        assert_eq!(s.target_kind, "StatefulSet");
        assert_eq!(s.trigger_types, vec!["cron", "kafka"]);
    }

    fn sum(cur: i32, min: i32, max: i32, paused: bool, found: bool) -> KedaResourceSummary {
        KedaResourceSummary {
            kind: "ScaledObject".into(),
            name: "n".into(),
            namespace: "d".into(),
            target_kind: "Deployment".into(),
            target_name: "n".into(),
            target_found: found,
            current_replicas: cur,
            min_replicas: min,
            max_replicas: max,
            paused,
            ready: true,
            trigger_types: vec![],
            age: String::new(),
        }
    }

    #[test]
    fn reasoning_rules_and_precedence() {
        use ScaleReasoningState::*;
        let st = |s| derive_scale_reasoning(&s).state;
        assert_eq!(st(sum(0, 0, 10, true, false)), TargetNotFound);
        assert_eq!(st(sum(0, 0, 10, true, true)), Paused);
        assert_eq!(st(sum(0, 0, 10, false, true)), ScaledToZero);
        assert_eq!(st(sum(1, 1, 10, false, true)), AtFloor);
        assert_eq!(st(sum(10, 1, 10, false, true)), AtCeiling);
        assert_eq!(st(sum(4, 1, 10, false, true)), MidRange);
        assert!(
            derive_scale_reasoning(&sum(1, 1, 10, false, true))
                .message
                .contains("floor")
        );
    }

    #[test]
    fn triggers_extract_auth_ref_and_threshold_without_secrets() {
        let o = obj(
            json!({"triggers": [{"type": "prometheus",
                "metadata": {"query": "up", "threshold": "50"},
                "authenticationRef": {"name": "creds", "kind": "ClusterTriggerAuthentication"}}]}),
            json!({}),
            None,
        );
        let t = extract_triggers(&o);
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].threshold.as_deref(), Some("50"));
        let a = t[0].auth_ref.as_ref().unwrap();
        assert_eq!((a.name.as_str(), a.cluster_scoped), ("creds", true));
        assert!(t[0].current_value.is_none());
    }

    struct Mock(Option<&'static str>);
    #[async_trait::async_trait]
    impl MetricsProviderTrait for Mock {
        async fn query_instant(
            &self,
            _q: &str,
            _t: Option<u64>,
        ) -> anyhow::Result<InstantQueryResponse> {
            match self.0 {
                Some(v) => Ok(InstantQueryResponse {
                    result_type: "vector".into(),
                    result: vec![MetricSample {
                        metric: BTreeMap::new(),
                        value: MetricValue {
                            timestamp: 0,
                            value: v.into(),
                        },
                    }],
                }),
                None => Err(anyhow::anyhow!("unreachable")),
            }
        }
        async fn query_range(&self, _: &str, _: u64, _: u64, _: &str) -> anyhow::Result<Value> {
            unimplemented!()
        }
        async fn get_metadata(
            &self,
            _: Option<&str>,
        ) -> anyhow::Result<BTreeMap<String, Vec<crate::backend::metrics_provider::MetricMetadata>>>
        {
            unimplemented!()
        }
    }

    fn trig(ty: &str) -> KedaTrigger {
        KedaTrigger {
            trigger_type: ty.into(),
            metadata: [("query".to_owned(), "up".to_owned())].into(),
            auth_ref: None,
            active: None,
            current_value: None,
            threshold: None,
        }
    }

    #[tokio::test]
    async fn prometheus_value_populated_and_failures_degrade() {
        let ok = Mock(Some("73"));
        let mut t = vec![trig("prometheus"), trig("kafka")];
        enrich_prometheus_values(&mut t, Some(&ok)).await;
        assert_eq!(t[0].current_value, Some(73.0));
        assert_eq!(t[1].current_value, None);

        let bad = Mock(None);
        let mut t = vec![trig("prometheus")];
        enrich_prometheus_values(&mut t, Some(&bad)).await;
        assert_eq!(t[0].current_value, None);

        let mut t = vec![trig("prometheus")];
        enrich_prometheus_values(&mut t, None).await;
        assert_eq!(t[0].current_value, None);
    }
}
