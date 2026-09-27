use super::served_crd::find_served;
use crate::protocol::DiskUsage;
use anyhow::Result;
use k8s_openapi::apiextensions_apiserver::pkg::apis::apiextensions::v1::CustomResourceDefinition;
use kube::api::ListParams;
use kube::core::DynamicObject;
use kube::{Client, api::Api};

const VOLUME_CRD: &str = "volumes.longhorn.io";

/// List Longhorn volumes and extract disk usage information.
pub(super) async fn list_longhorn_volumes(client: &Client) -> Result<Vec<DiskUsage>> {
    // Find a served version of volumes.longhorn.io; its plural comes from the CRD itself.
    let crd_api: Api<CustomResourceDefinition> = Api::all(client.clone());
    let Ok(list) = crd_api.list(&ListParams::default()).await else {
        return Ok(Vec::new());
    };
    let Some(crd) = find_served(&list.items, VOLUME_CRD) else {
        return Ok(Vec::new());
    };

    let api: Api<DynamicObject> = Api::all_with(client.clone(), &crd.api_resource());
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
                namespace,
                name,
                provisioned_bytes: spec_size,
                used_bytes: actual_size,
                state,
            });
        }
    }
    Ok(out)
}
