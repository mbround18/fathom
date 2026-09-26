use crate::protocol::{
    BackendEvent, BackendEventEnvelope, Correlation, ResourceListEvent, ResourceReadEvent, Route,
    SecretDetails, SecretSummary, section_keys,
};
use crossbeam_channel::Sender;
use k8s_openapi::api::core::v1::Secret;
use kube::{
    ResourceExt,
    api::{Api, DeleteParams, ListParams},
};

use super::interactions;
use super::{events, state::BackendState};

pub(super) async fn refresh_secrets(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
) -> anyhow::Result<()> {
    let client = state.cluster.client_or_err()?;
    let api: Api<Secret> = Api::all(client);

    let secrets = api.list(&ListParams::default()).await?;
    let summaries = secrets_to_summaries(secrets.items, &state.cluster.namespace_filter);
    events::emit(
        state.cluster.seq.as_ref(),
        sender,
        Route::namespaced(
            section_keys::config_secrets(),
            state.cluster.current_context.clone(),
            None,
        ),
        Correlation::None,
        BackendEvent::ResourceList(ResourceListEvent::SecretsUpdated(summaries)),
    );
    Ok(())
}

fn secrets_to_summaries(secrets: Vec<Secret>, filter: &[String]) -> Vec<SecretSummary> {
    let mut out: Vec<SecretSummary> = Vec::new();

    for secret in secrets {
        let namespace = secret.namespace().unwrap_or_default();
        if !filter.is_empty() && !filter.iter().any(|ns| ns == &namespace) {
            continue;
        }

        let name = secret.name_any();
        let secret_type = secret.type_.clone().unwrap_or_default();

        let keys_count = secret.data.as_ref().map(|d| d.len()).unwrap_or(0)
            + secret.string_data.as_ref().map(|d| d.len()).unwrap_or(0);

        out.push(SecretSummary {
            name,
            namespace,
            secret_type,
            keys_count,
        });
    }

    out.sort_by(|a, b| a.namespace.cmp(&b.namespace).then(a.name.cmp(&b.name)));
    out
}

pub(super) async fn fetch_secret_details(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    namespace: &str,
    name: &str,
) -> anyhow::Result<()> {
    let client = state.cluster.client_or_err()?;
    let api: Api<Secret> = Api::namespaced(client, namespace);
    let secret = api.get(name).await?;

    let details = secret_to_details(secret);
    events::emit(
        state.cluster.seq.as_ref(),
        sender,
        Route::namespaced(
            section_keys::config_secrets(),
            state.cluster.current_context.clone(),
            Some(namespace.to_string()),
        ),
        Correlation::None,
        BackendEvent::ResourceRead(ResourceReadEvent::SecretDetailsUpdated(details)),
    );
    Ok(())
}

const LAST_APPLIED: &str = "kubectl.kubernetes.io/last-applied-configuration";

/// Clones the secret and redacts all values in `.data` / `.string_data` so
/// the raw JSON never leaks decoded or base64 secret material.
fn redacted_raw_json(secret: &Secret) -> String {
    let mut redacted = secret.clone();

    // 040 FR-011: `kubectl apply` stores the whole manifest, plaintext values included, in
    // this annotation. Drop it so the raw view and exports never carry it.
    if let Some(annotations) = redacted.metadata.annotations.as_mut() {
        annotations.remove(LAST_APPLIED);
    }

    if let Some(data) = redacted.data.as_mut() {
        for value in data.values_mut() {
            *value = k8s_openapi::ByteString(b"<redacted>".to_vec());
        }
    }

    if let Some(string_data) = redacted.string_data.as_mut() {
        for value in string_data.values_mut() {
            *value = "<redacted>".to_string();
        }
    }

    serde_json::to_string_pretty(&redacted).unwrap_or_default()
}

// SECURITY: This function must never place decoded/raw secret values into
// `SecretDetails` — only key names and the secret type are exposed.
fn secret_to_details(secret: Secret) -> SecretDetails {
    let namespace = secret.namespace().unwrap_or_default();
    let name = secret.name_any();
    let creation_timestamp = secret
        .creation_timestamp()
        .map(|ts| ts.0.to_rfc3339())
        .unwrap_or_default();

    let labels = secret.labels().clone();
    let mut annotations = secret.annotations().clone();
    annotations.remove(LAST_APPLIED);
    let secret_type = secret.type_.clone().unwrap_or_default();

    let mut keys: Vec<String> = Vec::new();
    if let Some(data) = secret.data.as_ref() {
        keys.extend(data.keys().cloned());
    }
    if let Some(string_data) = secret.string_data.as_ref() {
        keys.extend(string_data.keys().cloned());
    }
    keys.sort();
    keys.dedup();

    let raw_json = redacted_raw_json(&secret);

    SecretDetails {
        namespace,
        name,
        creation_timestamp,
        labels,
        annotations,
        secret_type,
        keys,
        raw_json,
    }
}

pub(super) async fn delete_secret(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    namespace: &str,
    name: &str,
) -> anyhow::Result<()> {
    let client = state.cluster.client_or_err()?;
    let api: Api<Secret> = Api::namespaced(client, namespace);

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

    refresh_secrets(state, sender).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn applied_secret() -> Secret {
        let mut s = Secret::default();
        s.metadata.name = Some("db".into());
        s.metadata.namespace = Some("default".into());
        s.metadata.annotations = Some(BTreeMap::from([
            (
                LAST_APPLIED.to_owned(),
                r#"{"stringData":{"password":"hunter2"}}"#.to_owned(),
            ),
            ("team".to_owned(), "a".to_owned()),
        ]));
        s.data = Some(BTreeMap::from([(
            "password".to_owned(),
            k8s_openapi::ByteString(b"hunter2".to_vec()),
        )]));
        s
    }

    #[test]
    fn raw_json_drops_values_and_last_applied() {
        let raw = redacted_raw_json(&applied_secret());
        assert!(!raw.contains("hunter2"), "{raw}");
        assert!(!raw.contains(LAST_APPLIED), "{raw}");
        assert!(raw.contains("\"team\""), "{raw}");
    }

    #[test]
    fn details_annotations_drop_last_applied() {
        let details = secret_to_details(applied_secret());
        assert!(!details.annotations.contains_key(LAST_APPLIED));
        assert!(!details.raw_json.contains("hunter2"));
    }
}
