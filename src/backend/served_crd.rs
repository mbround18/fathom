//! The REST identity of a served CRD, taken from the cluster rather than guessed.
//!
//! `ApiResource::from_gvk` derives the URL plural from the kind with English rules, which is
//! wrong for any CRD whose declared plural does not follow them: Gatekeeper's `K8sAllowedRepos`
//! became `k8sallowedreposes` and its scan 404ed. A CRD declares its real plural in
//! `spec.names.plural` (the same name API discovery reports), so the integration scanners that
//! already LIST CRDs read it from there: no extra round-trip and nothing to cache.
//!
//! Callers that only have an `apiVersion`/`kind` pair (YAML apply, manifest export, the MCP
//! tools) resolve it through API discovery instead, with `apply::resolve_api_resource`.

use k8s_openapi::apiextensions_apiserver::pkg::apis::apiextensions::v1::CustomResourceDefinition;
use kube::core::{ApiResource, GroupVersionKind};

/// A CRD that is installed and has at least one served version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ServedCrd {
    pub group: String,
    pub version: String,
    pub kind: String,
    /// The resource name from the CRD's `spec.names.plural`, never derived from `kind`.
    pub plural: String,
}

impl ServedCrd {
    /// The `ApiResource` for this CRD's instances, using the declared plural.
    pub(crate) fn api_resource(&self) -> ApiResource {
        let gvk = GroupVersionKind::gvk(&self.group, &self.version, &self.kind);
        ApiResource::from_gvk_with_plural(&gvk, &self.plural)
    }
}

/// Find `crd_name` (`<plural>.<group>`) among already-listed CRDs and return its first served
/// version; `None` when it is not installed or no version is served.
pub(crate) fn find_served(crds: &[CustomResourceDefinition], crd_name: &str) -> Option<ServedCrd> {
    let crd = crds
        .iter()
        .find(|c| c.metadata.name.as_deref() == Some(crd_name))?;
    let version = crd
        .spec
        .versions
        .iter()
        .find(|v| v.served)
        .map(|v| v.name.clone())?;
    Some(ServedCrd {
        group: crd.spec.group.clone(),
        version,
        kind: crd.spec.names.kind.clone(),
        plural: crd.spec.names.plural.clone(),
    })
}

/// A fake API server answering from `route(path)` with `(status, JSON body)`. Shared by the
/// scanner tests so each one exercises the real URL the backend builds.
#[cfg(test)]
pub(crate) fn fake_client(route: fn(&str) -> (u16, serde_json::Value)) -> kube::Client {
    let service = tower::service_fn(move |req: http::Request<kube::client::Body>| {
        let (status, body) = route(req.uri().path());
        async move {
            Ok::<_, std::convert::Infallible>(
                http::Response::builder()
                    .status(status)
                    .header("content-type", "application/json")
                    .body(kube::client::Body::from(body.to_string().into_bytes()))
                    .expect("response"),
            )
        }
    });
    kube::Client::new(service, "default")
}

#[cfg(test)]
mod tests {
    use super::*;
    use kube::api::{Api, ListParams};
    use kube::core::DynamicObject;
    use serde_json::{Value, json};

    fn crd(group: &str, kind: &str, plural: &str, versions: &[(&str, bool)]) -> Value {
        json!({
            "apiVersion": "apiextensions.k8s.io/v1",
            "kind": "CustomResourceDefinition",
            "metadata": { "name": format!("{plural}.{group}") },
            "spec": {
                "group": group,
                "scope": "Namespaced",
                "names": { "kind": kind, "plural": plural, "listKind": format!("{kind}List") },
                "versions": versions.iter().map(|(name, served)| json!({
                    "name": name, "served": served, "storage": served
                })).collect::<Vec<_>>()
            }
        })
    }

    fn crds(items: &[Value]) -> Vec<CustomResourceDefinition> {
        items
            .iter()
            .map(|v| serde_json::from_value(v.clone()).expect("crd"))
            .collect()
    }

    #[test]
    fn plural_comes_from_the_crd_not_the_kind() {
        let list = crds(&[
            crd(
                "constraints.gatekeeper.sh",
                "K8sAllowedRepos",
                "k8sallowedrepos",
                &[("v1beta1", true)],
            ),
            crd("example.io", "Chaos", "chaos", &[("v1", true)]),
        ]);
        let repos = find_served(&list, "k8sallowedrepos.constraints.gatekeeper.sh").expect("found");
        assert_eq!(repos.plural, "k8sallowedrepos");
        assert_eq!(repos.api_resource().plural, "k8sallowedrepos");
        assert_eq!(
            repos.api_resource().api_version,
            "constraints.gatekeeper.sh/v1beta1"
        );

        let chaos = find_served(&list, "chaos.example.io").expect("found");
        assert_eq!(chaos.api_resource().plural, "chaos");
    }

    #[test]
    fn the_kind_based_guess_is_wrong_for_these_plurals() {
        // Documents why `from_gvk` must not be used for CRDs: its guess differs.
        let gvk = GroupVersionKind::gvk("constraints.gatekeeper.sh", "v1beta1", "K8sAllowedRepos");
        assert_eq!(ApiResource::from_gvk(&gvk).plural, "k8sallowedreposes");
        let gvk = GroupVersionKind::gvk("example.io", "v1", "Chaos");
        assert_eq!(ApiResource::from_gvk(&gvk).plural, "chaoses");
    }

    #[test]
    fn picks_the_first_served_version() {
        let list = crds(&[crd(
            "example.io",
            "Widget",
            "widgets",
            &[("v1alpha1", false), ("v1beta1", true), ("v1", true)],
        )]);
        let widget = find_served(&list, "widgets.example.io").expect("found");
        assert_eq!(widget.version, "v1beta1");
        assert_eq!(widget.group, "example.io");
        assert_eq!(widget.kind, "Widget");
    }

    #[test]
    fn none_when_missing_or_unserved() {
        let list = crds(&[crd("example.io", "Widget", "widgets", &[("v1", false)])]);
        assert!(find_served(&list, "widgets.example.io").is_none());
        assert!(find_served(&list, "gadgets.example.io").is_none());
        assert!(find_served(&[], "widgets.example.io").is_none());
    }

    /// Regression: an instance LIST built from a listed CRD hits the declared plural. The fake
    /// server 404s every other path, as a real API server does for the guessed `chaoses`.
    #[tokio::test]
    async fn instance_list_uses_the_declared_plural() {
        fn route(path: &str) -> (u16, Value) {
            match path {
                "/apis/example.io/v1/chaos" => (
                    200,
                    json!({ "apiVersion": "example.io/v1", "kind": "ChaosList", "metadata": {},
                            "items": [ { "apiVersion": "example.io/v1", "kind": "Chaos",
                                         "metadata": { "name": "monkey", "namespace": "default" } } ] }),
                ),
                other => (
                    404,
                    json!({ "kind": "Status", "apiVersion": "v1", "status": "Failure",
                            "code": 404, "message": format!("not found: {other}") }),
                ),
            }
        }
        let client = fake_client(route);
        let list = crds(&[crd("example.io", "Chaos", "chaos", &[("v1", true)])]);
        let chaos = find_served(&list, "chaos.example.io").expect("found");

        let api: Api<DynamicObject> = Api::all_with(client.clone(), &chaos.api_resource());
        let items = api.list(&ListParams::default()).await.expect("list").items;
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].metadata.name.as_deref(), Some("monkey"));

        // The guessed plural is exactly what used to fail.
        let guessed = ApiResource::from_gvk(&GroupVersionKind::gvk("example.io", "v1", "Chaos"));
        let api: Api<DynamicObject> = Api::all_with(client, &guessed);
        assert!(api.list(&ListParams::default()).await.is_err());
    }
}
