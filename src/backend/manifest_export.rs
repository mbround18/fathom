//! 040-manifest-export: read objects fresh from the cluster for YAML export.
//!
//! The backend only fetches; cleaning and YAML rendering happen in the UI (research R1), so
//! the same code runs against the fake transport in e2e. The one exception is Secrets: their
//! values are redacted here, before anything crosses the protocol (FR-011, constitution VI).

use std::collections::HashMap;

use futures::StreamExt;
use kube::Client;
use kube::api::{Api, DynamicObject};
use kube::core::ApiResource;
use serde_json::Value;

use super::apply;
use crate::protocol::{
    FetchedManifest, MANIFEST_EXPORT_MAX_OBJECTS, ManifestFetchError, ManifestObjectRef,
};

/// Parallel GETs per request. Keeps a 500-object bundle quick without hammering the API server.
const CONCURRENCY: usize = 8;

const LAST_APPLIED: &str = "kubectl.kubernetes.io/last-applied-configuration";
const REDACTED: &str = "<redacted>";

/// Fetch every object in `objects`. Each index ends up in exactly one of the two vectors,
/// both sorted by index. Over the cap, nothing is fetched and a single error is returned.
pub(super) async fn fetch_manifests(
    client: &Client,
    objects: &[ManifestObjectRef],
) -> (Vec<FetchedManifest>, Vec<ManifestFetchError>) {
    if let Some(err) = over_cap(objects.len()) {
        return (Vec::new(), vec![err]);
    }

    // One discovery round-trip per distinct apiVersion/kind.
    let mut resources: HashMap<(String, String), Result<ApiResource, String>> = HashMap::new();
    for obj in objects {
        let key = (obj.api_version.clone(), obj.kind.clone());
        if resources.contains_key(&key) {
            continue;
        }
        let resolved = apply::resolve_api_resource(client, &obj.api_version, &obj.kind)
            .await
            .map_err(|e| format!("{e:#}"));
        resources.insert(key, resolved);
    }

    // Owned jobs: the stream must not borrow from the caller (the future is spawned).
    let jobs: Vec<(u32, ManifestObjectRef, Result<ApiResource, String>)> = objects
        .iter()
        .enumerate()
        .map(|(i, obj)| {
            let resource = resources
                .get(&(obj.api_version.clone(), obj.kind.clone()))
                .cloned()
                .unwrap_or_else(|| Err("not discovered".to_owned()));
            (u32::try_from(i).unwrap_or(u32::MAX), obj.clone(), resource)
        })
        .collect();
    let results: Vec<(u32, Result<String, String>)> = futures::stream::iter(jobs)
        .map(|(index, obj, resource)| {
            let client = client.clone();
            async move {
                let out = match resource {
                    Ok(ar) => fetch_one(client, ar, obj).await,
                    Err(e) => Err(e),
                };
                (index, out)
            }
        })
        .buffer_unordered(CONCURRENCY)
        .collect()
        .await;

    split_results(results)
}

fn over_cap(len: usize) -> Option<ManifestFetchError> {
    (len > MANIFEST_EXPORT_MAX_OBJECTS).then(|| ManifestFetchError {
        index: 0,
        message: format!(
            "too many objects: {len} selected, the export limit is {MANIFEST_EXPORT_MAX_OBJECTS}"
        ),
    })
}

fn split_results(
    mut results: Vec<(u32, Result<String, String>)>,
) -> (Vec<FetchedManifest>, Vec<ManifestFetchError>) {
    results.sort_by_key(|(i, _)| *i);
    let mut manifests = Vec::new();
    let mut errors = Vec::new();
    for (index, r) in results {
        match r {
            Ok(raw_json) => manifests.push(FetchedManifest { index, raw_json }),
            Err(message) => errors.push(ManifestFetchError { index, message }),
        }
    }
    (manifests, errors)
}

async fn fetch_one(
    client: Client,
    ar: ApiResource,
    obj: ManifestObjectRef,
) -> Result<String, String> {
    let api: Api<DynamicObject> = match obj.namespace.as_deref() {
        Some(ns) => Api::namespaced_with(client, ns, &ar),
        None => Api::all_with(client, &ar),
    };
    let found = api.get(&obj.name).await.map_err(|e| match e {
        kube::Error::Api(resp) => resp.message,
        other => other.to_string(),
    })?;
    let mut value = serde_json::to_value(&found).map_err(|e| e.to_string())?;
    prepare_for_export(&obj.api_version, &obj.kind, &mut value);
    serde_json::to_string(&value).map_err(|e| e.to_string())
}

/// Fill in missing type metadata and apply the Secret rules (FR-011): every `data` and
/// `stringData` value becomes `<redacted>`, and the last-applied annotation (which can carry
/// the plaintext values) is dropped.
pub(super) fn prepare_for_export(api_version: &str, kind: &str, value: &mut Value) {
    let Some(map) = value.as_object_mut() else {
        return;
    };
    map.entry("apiVersion")
        .or_insert_with(|| Value::String(api_version.to_owned()));
    map.entry("kind")
        .or_insert_with(|| Value::String(kind.to_owned()));
    if kind == "Secret" && api_version == "v1" {
        redact_secret(value);
    }
}

pub(super) fn redact_secret(value: &mut Value) {
    for field in ["data", "stringData"] {
        if let Some(Value::Object(entries)) = value.get_mut(field) {
            for v in entries.values_mut() {
                *v = Value::String(REDACTED.to_owned());
            }
        }
    }
    if let Some(Value::Object(annotations)) = value
        .get_mut("metadata")
        .and_then(|m| m.get_mut("annotations"))
    {
        annotations.remove(LAST_APPLIED);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn secret_values_and_last_applied_are_removed() {
        let mut v = json!({
            "apiVersion": "v1",
            "kind": "Secret",
            "metadata": {
                "name": "s",
                "annotations": {
                    LAST_APPLIED: "{\"data\":{\"password\":\"aHVudGVyMg==\"}}",
                    "keep": "me"
                }
            },
            "data": {"password": "aHVudGVyMg=="},
            "stringData": {"token": "plain"}
        });
        prepare_for_export("v1", "Secret", &mut v);
        let s = v.to_string();
        assert!(!s.contains("aHVudGVyMg=="), "{s}");
        assert!(!s.contains("plain"), "{s}");
        assert_eq!(v["data"]["password"], "<redacted>");
        assert_eq!(v["stringData"]["token"], "<redacted>");
        assert_eq!(v["metadata"]["annotations"]["keep"], "me");
        assert!(v["metadata"]["annotations"].get(LAST_APPLIED).is_none());
    }

    #[test]
    fn non_secret_objects_are_left_alone_and_type_meta_filled() {
        let mut v = json!({
            "metadata": {"name": "c", "annotations": {LAST_APPLIED: "{}"}},
            "data": {"k": "v"}
        });
        prepare_for_export("v1", "ConfigMap", &mut v);
        assert_eq!(v["apiVersion"], "v1");
        assert_eq!(v["kind"], "ConfigMap");
        assert_eq!(v["data"]["k"], "v");
        assert!(v["metadata"]["annotations"].get(LAST_APPLIED).is_some());
    }

    #[test]
    fn a_crd_named_secret_is_not_redacted() {
        let mut v = json!({"data": {"k": "v"}});
        prepare_for_export("example.com/v1", "Secret", &mut v);
        assert_eq!(v["data"]["k"], "v");
    }

    #[test]
    fn cap_is_enforced() {
        assert!(over_cap(MANIFEST_EXPORT_MAX_OBJECTS).is_none());
        let err = over_cap(MANIFEST_EXPORT_MAX_OBJECTS + 1).unwrap();
        assert_eq!(err.index, 0);
        assert!(err.message.contains("too many objects"));
    }

    #[test]
    fn results_are_split_and_sorted_by_index() {
        let (m, e) = split_results(vec![
            (2, Ok("c".into())),
            (0, Ok("a".into())),
            (1, Err("gone".into())),
        ]);
        assert_eq!(m.iter().map(|x| x.index).collect::<Vec<_>>(), [0, 2]);
        assert_eq!(e.len(), 1);
        assert_eq!(e[0].index, 1);
        assert_eq!(e[0].message, "gone");
    }
}
