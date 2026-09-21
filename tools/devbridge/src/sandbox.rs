//! Startup check that the write namespace really is the labelled dev sandbox.

use anyhow::Context as _;
use k8s_openapi::api::core::v1::Namespace;
use kube::config::KubeConfigOptions;
use kube::{Api, Client, Config};
use std::collections::BTreeMap;

/// The label `dev/sandbox/namespace.yaml` puts on the sandbox namespace.
pub const SANDBOX_LABEL_KEY: &str = "fathom.dev/sandbox";
pub const SANDBOX_LABEL_VALUE: &str = "true";

/// `Ok` if the namespace's labels mark it as the sandbox.
pub fn check_sandbox_labels(
    namespace: &str,
    labels: Option<&BTreeMap<String, String>>,
) -> Result<(), String> {
    match labels.and_then(|l| l.get(SANDBOX_LABEL_KEY)) {
        Some(v) if v == SANDBOX_LABEL_VALUE => Ok(()),
        Some(v) => Err(format!(
            "namespace '{namespace}' has {SANDBOX_LABEL_KEY}={v}, expected {SANDBOX_LABEL_VALUE}"
        )),
        None => Err(format!(
            "namespace '{namespace}' is not labelled {SANDBOX_LABEL_KEY}={SANDBOX_LABEL_VALUE}; \
             refusing to allow writes to it"
        )),
    }
}

/// Read-only: fetch `namespace` in kube `context` and require the sandbox label.
pub async fn verify_sandbox(context: &str, namespace: &str) -> anyhow::Result<()> {
    let options = KubeConfigOptions {
        context: Some(context.to_owned()),
        ..KubeConfigOptions::default()
    };
    let config = Config::from_kubeconfig(&options)
        .await
        .with_context(|| format!("loading kubeconfig context '{context}'"))?;
    let client = Client::try_from(config).context("building the Kubernetes client")?;
    let ns = Api::<Namespace>::all(client)
        .get_opt(namespace)
        .await
        .with_context(|| format!("reading namespace '{namespace}' in context '{context}'"))?
        .with_context(|| {
            format!("namespace '{namespace}' does not exist in context '{context}'")
        })?;
    check_sandbox_labels(namespace, ns.metadata.labels.as_ref()).map_err(anyhow::Error::msg)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }

    #[test]
    fn labelled_namespace_passes() {
        let l = labels(&[("fathom.dev/sandbox", "true"), ("x", "y")]);
        assert_eq!(check_sandbox_labels("fathom-dev", Some(&l)), Ok(()));
    }

    #[test]
    fn missing_or_wrong_label_fails() {
        assert!(check_sandbox_labels("fathom-dev", None).is_err());
        let other = labels(&[("x", "y")]);
        assert!(check_sandbox_labels("fathom-dev", Some(&other)).is_err());
        let wrong = labels(&[("fathom.dev/sandbox", "false")]);
        assert!(check_sandbox_labels("fathom-dev", Some(&wrong)).is_err());
    }
}
