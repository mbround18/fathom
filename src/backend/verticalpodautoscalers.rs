//! VerticalPodAutoscaler (`autoscaling.k8s.io/v1`, a CRD). Read as dynamic objects; when the CRD
//! is not installed the API server answers 404 and the error propagates as a list error, which
//! the UI renders as "not available".

use crate::protocol::{
    BackendEvent, BackendEventEnvelope, Correlation, ResourceListEvent, ResourceReadEvent, Route,
    VerticalPodAutoscalerDetails, VerticalPodAutoscalerSummary, VpaContainerRecommendation,
    VpaResources, section_keys,
};
use crossbeam_channel::Sender;
use kube::{
    ResourceExt,
    api::{Api, DeleteParams},
    core::{ApiResource, DynamicObject, GroupVersionKind},
};
use serde_json::Value;

use super::interactions;
use super::{events, state::BackendState};

fn api_resource() -> ApiResource {
    ApiResource::from_gvk(&GroupVersionKind::gvk(
        "autoscaling.k8s.io",
        "v1",
        "VerticalPodAutoscaler",
    ))
}

pub(super) async fn refresh_verticalpodautoscalers(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
) -> anyhow::Result<()> {
    let client = state.cluster.client_or_err()?;
    let vpas = super::ns_scope::list_scoped_dynamic(
        client,
        &api_resource(),
        &state.cluster.namespace_filter,
        &state.cluster.scope.snapshot(),
    )
    .await?;
    let summaries = vpas_to_summaries(vpas, &state.cluster.namespace_filter);
    events::emit(
        state.cluster.seq.as_ref(),
        sender,
        Route::namespaced(
            section_keys::config_verticalpodautoscalers(),
            state.cluster.current_context.clone(),
            None,
        ),
        Correlation::None,
        BackendEvent::ResourceList(ResourceListEvent::VerticalPodAutoscalersUpdated(summaries)),
    );
    Ok(())
}

fn str_at(v: &Value, key: &str) -> String {
    v.get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn resources(v: Option<&Value>) -> VpaResources {
    let v = v.cloned().unwrap_or(Value::Null);
    VpaResources {
        cpu: str_at(&v, "cpu"),
        memory: str_at(&v, "memory"),
    }
}

fn target_ref(data: &Value) -> String {
    let t = data.pointer("/spec/targetRef").unwrap_or(&Value::Null);
    let (kind, name) = (str_at(t, "kind"), str_at(t, "name"));
    if kind.is_empty() && name.is_empty() {
        String::new()
    } else {
        format!("{kind}/{name}")
    }
}

fn update_mode(data: &Value) -> String {
    data.pointer("/spec/updatePolicy/updateMode")
        .and_then(Value::as_str)
        .unwrap_or("Auto")
        .to_string()
}

fn recommendations(data: &Value) -> Vec<VpaContainerRecommendation> {
    data.pointer("/status/recommendation/containerRecommendations")
        .and_then(Value::as_array)
        .map(|recs| {
            recs.iter()
                .map(|r| VpaContainerRecommendation {
                    container: str_at(r, "containerName"),
                    lower_bound: resources(r.get("lowerBound")),
                    target: resources(r.get("target")),
                    upper_bound: resources(r.get("upperBound")),
                })
                .collect()
        })
        .unwrap_or_default()
}

fn conditions(data: &Value) -> Vec<String> {
    data.pointer("/status/conditions")
        .and_then(Value::as_array)
        .map(|cs| {
            cs.iter()
                .map(|c| {
                    let base = format!("{}={}", str_at(c, "type"), str_at(c, "status"));
                    match str_at(c, "reason") {
                        r if r.is_empty() => base,
                        r => format!("{base} ({r})"),
                    }
                })
                .collect()
        })
        .unwrap_or_default()
}

fn vpas_to_summaries(
    vpas: Vec<DynamicObject>,
    filter: &[String],
) -> Vec<VerticalPodAutoscalerSummary> {
    let mut out: Vec<VerticalPodAutoscalerSummary> = Vec::new();
    for vpa in vpas {
        let namespace = vpa.namespace().unwrap_or_default();
        if !filter.is_empty() && !filter.iter().any(|ns| ns == &namespace) {
            continue;
        }
        out.push(VerticalPodAutoscalerSummary {
            namespace,
            name: vpa.name_any(),
            target_ref: target_ref(&vpa.data),
            update_mode: update_mode(&vpa.data),
            recommendations: recommendations(&vpa.data),
        });
    }
    out.sort_by(|a, b| a.namespace.cmp(&b.namespace).then(a.name.cmp(&b.name)));
    out
}

pub(super) async fn fetch_verticalpodautoscaler_details(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    namespace: &str,
    name: &str,
) -> anyhow::Result<()> {
    let client = state.cluster.client_or_err()?;
    let api: Api<DynamicObject> = Api::namespaced_with(client, namespace, &api_resource());
    let vpa = api.get(name).await?;

    let details = vpa_to_details(vpa);
    events::emit(
        state.cluster.seq.as_ref(),
        sender,
        Route::namespaced(
            section_keys::config_verticalpodautoscalers(),
            state.cluster.current_context.clone(),
            Some(namespace.to_string()),
        ),
        Correlation::None,
        BackendEvent::ResourceRead(ResourceReadEvent::VerticalPodAutoscalerDetailsUpdated(
            details,
        )),
    );
    Ok(())
}

fn vpa_to_details(vpa: DynamicObject) -> VerticalPodAutoscalerDetails {
    VerticalPodAutoscalerDetails {
        namespace: vpa.namespace().unwrap_or_default(),
        name: vpa.name_any(),
        creation_timestamp: vpa
            .creation_timestamp()
            .map(|ts| ts.0.to_rfc3339())
            .unwrap_or_default(),
        labels: vpa.labels().clone(),
        annotations: vpa.annotations().clone(),
        target_ref: target_ref(&vpa.data),
        update_mode: update_mode(&vpa.data),
        recommendations: recommendations(&vpa.data),
        conditions: conditions(&vpa.data),
        raw_json: serde_json::to_string_pretty(&vpa).unwrap_or_default(),
    }
}

pub(super) async fn delete_verticalpodautoscaler(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    namespace: &str,
    name: &str,
) -> anyhow::Result<()> {
    let client = state.cluster.client_or_err()?;
    let api: Api<DynamicObject> = Api::namespaced_with(client, namespace, &api_resource());

    let ts = k8s_openapi::chrono::Utc::now().to_rfc3339();
    let _ = interactions::annotate_action(
        &api,
        name,
        state.cluster.current_context.as_deref(),
        "delete",
        &ts,
        serde_json::json!({}),
    )
    .await;

    api.delete(name, &DeleteParams::default()).await?;

    refresh_verticalpodautoscalers(state, sender).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn vpa(ns: &str, name: &str, spec_mode: Option<&str>) -> DynamicObject {
        let mut spec = json!({"targetRef": {"kind": "Deployment", "name": "web"}});
        if let Some(m) = spec_mode {
            spec["updatePolicy"] = json!({"updateMode": m});
        }
        serde_json::from_value(json!({
            "apiVersion": "autoscaling.k8s.io/v1",
            "kind": "VerticalPodAutoscaler",
            "metadata": {"namespace": ns, "name": name},
            "spec": spec,
            "status": {
                "recommendation": {"containerRecommendations": [{
                    "containerName": "app",
                    "lowerBound": {"cpu": "25m", "memory": "100Mi"},
                    "target": {"cpu": "50m", "memory": "200Mi"},
                    "upperBound": {"cpu": "1", "memory": "1Gi"}
                }]},
                "conditions": [{"type": "RecommendationProvided", "status": "True", "reason": "Ok"}]
            }
        }))
        .unwrap()
    }

    #[test]
    fn summarizes_target_mode_and_recommendations() {
        let out = vpas_to_summaries(vec![vpa("b", "z", Some("Off")), vpa("a", "y", None)], &[]);
        assert_eq!(out[0].namespace, "a");
        assert_eq!(out[0].update_mode, "Auto");
        assert_eq!(out[1].update_mode, "Off");
        assert_eq!(out[0].target_ref, "Deployment/web");
        let r = &out[0].recommendations[0];
        assert_eq!(r.container, "app");
        assert_eq!(r.lower_bound.cpu, "25m");
        assert_eq!(r.target.memory, "200Mi");
        assert_eq!(r.upper_bound.cpu, "1");
    }

    #[test]
    fn namespace_filter_applies() {
        let out = vpas_to_summaries(
            vec![vpa("a", "x", None), vpa("b", "y", None)],
            &["b".to_string()],
        );
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].name, "y");
    }

    #[test]
    fn details_include_conditions_and_raw_json() {
        let d = vpa_to_details(vpa("a", "x", Some("Recreate")));
        assert_eq!(d.conditions, vec!["RecommendationProvided=True (Ok)"]);
        assert_eq!(d.update_mode, "Recreate");
        assert!(d.raw_json.contains("containerRecommendations"));
    }

    #[test]
    fn missing_status_yields_empty() {
        let o: DynamicObject = serde_json::from_value(json!({
            "apiVersion": "autoscaling.k8s.io/v1", "kind": "VerticalPodAutoscaler",
            "metadata": {"namespace": "a", "name": "x"}
        }))
        .unwrap();
        assert!(recommendations(&o.data).is_empty());
        assert_eq!(target_ref(&o.data), "");
    }
}
