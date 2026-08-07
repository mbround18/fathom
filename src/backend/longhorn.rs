use crate::protocol::DiskUsage;
use anyhow::Result;
use kube::api::ListParams;
use kube::core::DynamicObject;
use kube::core::{ApiResource, GroupVersionKind};
use kube::{Client, api::Api};

/// List Longhorn volumes and extract disk usage information.
pub(super) async fn list_longhorn_volumes(client: &Client) -> Result<Vec<DiskUsage>> {
    // Discover the CRD version and kind for volumes.longhorn.io
    // We'll look up the CRD named "volumes.longhorn.io"
    use k8s_openapi::apiextensions_apiserver::pkg::apis::apiextensions::v1::CustomResourceDefinition;
    let crd_api: Api<CustomResourceDefinition> = Api::all(client.clone());
    if let Ok(list) = crd_api.list(&ListParams::default()).await {
        for crd in list.items {
            if let Some(name) = crd.metadata.name.as_ref()
                && name == "volumes.longhorn.io"
            {
                // Find a served version
                let spec = crd.spec;
                let group = spec.group;
                if let Some(version) = spec
                    .versions
                    .iter()
                    .find(|v| v.served)
                    .map(|v| v.name.clone())
                {
                    let kind = spec.names.kind;
                    let gvk = GroupVersionKind::gvk(&group, &version, &kind);
                    let resource = ApiResource::from_gvk(&gvk);
                    let api: Api<DynamicObject> = Api::all_with(client.clone(), &resource);
                    let mut out = Vec::new();
                    if let Ok(volumes) = api.list(&ListParams::default()).await {
                        for vol in volumes.items {
                            let name = vol.metadata.name.clone().unwrap_or_default();
                            let namespace = vol.metadata.namespace.clone().unwrap_or_default();
                            let spec_size = vol
                                .data
                                .get("spec")
                                .and_then(|s| s.get("size"))
                                .and_then(|v| v.as_str())
                                .and_then(|s| s.parse::<u64>().ok())
                                .unwrap_or(0);
                            let actual_size = vol
                                .data
                                .get("status")
                                .and_then(|s| s.get("actualSize"))
                                .and_then(|v| v.as_u64());
                            let state = vol
                                .data
                                .get("status")
                                .and_then(|s| s.get("state"))
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string());
                            out.push(DiskUsage {
                                namespace: namespace.clone(),
                                name: name.clone(),
                                provisioned_bytes: spec_size,
                                used_bytes: actual_size,
                                state,
                            });
                        }
                    }
                    return Ok(out);
                }
            }
        }
    }

    Ok(Vec::new())
}
