use kube::api::{Api, Patch, PatchParams};
use kube::config::Kubeconfig;
use serde::{Deserialize, Serialize};
use serde_json::json;

const INTERACTIONS_ANNOTATION: &str = "fathom/interactions";
const MAX_ITEMS: usize = 50;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct InteractionLog {
    v: u8,
    items: Vec<InteractionItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct InteractionItem {
    ts: String,
    action: String,

    #[serde(skip_serializing_if = "Option::is_none")]
    user: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    context: Option<String>,

    #[serde(default)]
    details: serde_json::Value,
}

fn best_effort_user_for_context(context: Option<&str>) -> Option<String> {
    let kubeconfig = Kubeconfig::read().ok()?;
    let ctx_name = context
        .map(|s| s.to_owned())
        .or_else(|| kubeconfig.current_context.clone())?;

    let ctx = kubeconfig.contexts.iter().find(|c| c.name == ctx_name)?;
    // `context` is an inner struct on kubeconfig contexts.
    ctx.context.as_ref().and_then(|c| c.user.clone())
}

fn append_interaction(existing: Option<&str>, item: InteractionItem) -> String {
    let mut log = existing
        .and_then(|s| serde_json::from_str::<InteractionLog>(s).ok())
        .unwrap_or(InteractionLog {
            v: 1,
            items: Vec::new(),
        });

    log.items.push(item);
    if log.items.len() > MAX_ITEMS {
        let drain = log.items.len().saturating_sub(MAX_ITEMS);
        log.items.drain(0..drain);
    }

    // Compact JSON is smaller and keeps annotation overhead down.
    serde_json::to_string(&log).unwrap_or_else(|_| "{\"v\":1,\"items\":[]}".to_owned())
}

pub(super) async fn annotate_action<T>(
    api: &Api<T>,
    name: &str,
    context: Option<&str>,
    action: &str,
    ts_rfc3339: &str,
    details: serde_json::Value,
) -> anyhow::Result<()>
where
    T: std::fmt::Debug + Clone + serde::de::DeserializeOwned + serde::Serialize + kube::ResourceExt,
{
    let user = best_effort_user_for_context(context);

    let obj = api.get(name).await?;
    let existing = obj
        .annotations()
        .get(INTERACTIONS_ANNOTATION)
        .map(|s| s.as_str());

    let value = append_interaction(
        existing,
        InteractionItem {
            ts: ts_rfc3339.to_owned(),
            action: action.to_owned(),
            user,
            context: context.map(|s| s.to_owned()),
            details,
        },
    );

    let patch = json!({
        "metadata": {"annotations": {INTERACTIONS_ANNOTATION: value}}
    });

    api.patch(name, &PatchParams::default(), &Patch::Merge(&patch))
        .await?;

    Ok(())
}
