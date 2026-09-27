use crate::protocol::{
    BackendEvent, BackendEventEnvelope, Correlation, CronJobDetails, CronJobSummary,
    ResourceListEvent, ResourceReadEvent, Route, section_keys,
};
use crossbeam_channel::Sender;
use k8s_openapi::{api::batch::v1::CronJob, chrono::Utc};
use kube::ResourceExt;
use kube::api::{Api, DeleteParams, Patch, PatchParams};
use serde_json::json;

use super::interactions;
use super::{events, state::BackendState};

pub(super) async fn refresh_cronjobs(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
) -> anyhow::Result<()> {
    let cronjobs = state.cluster.list_namespaced::<CronJob>().await?;
    let summaries = cronjobs_to_summaries(cronjobs, &state.cluster.namespace_filter);

    events::emit(
        state.cluster.seq.as_ref(),
        sender,
        Route::namespaced(
            section_keys::workloads_cronjobs(),
            state.cluster.current_context.clone(),
            None,
        ),
        Correlation::None,
        BackendEvent::ResourceList(ResourceListEvent::CronJobsUpdated(summaries)),
    );
    Ok(())
}

fn cronjobs_to_summaries(cronjobs: Vec<CronJob>, filter: &[String]) -> Vec<CronJobSummary> {
    let mut out: Vec<CronJobSummary> = Vec::new();

    for cj in cronjobs {
        let namespace = cj.namespace().unwrap_or_default();
        if !filter.is_empty() && !filter.iter().any(|ns| ns == &namespace) {
            continue;
        }

        let name = cj.name_any();
        let schedule = cj
            .spec
            .as_ref()
            .map(|s| s.schedule.clone())
            .unwrap_or_else(|| "-".to_string());
        let suspend = cj.spec.as_ref().and_then(|s| s.suspend).unwrap_or(false);
        let active = cj
            .status
            .as_ref()
            .and_then(|s| s.active.as_ref())
            .map(|a| a.len() as i32)
            .unwrap_or(0);

        let last_schedule = cj
            .status
            .as_ref()
            .and_then(|s| s.last_schedule_time.as_ref())
            .map(|ts| {
                let now = Utc::now();
                let dur = now.signed_duration_since(ts.0);
                format_duration(dur.num_seconds())
            });

        let age = cj
            .metadata
            .creation_timestamp
            .as_ref()
            .map(|ts| {
                let now = Utc::now();
                let dur = now.signed_duration_since(ts.0);
                format_duration(dur.num_seconds())
            })
            .unwrap_or_else(|| "-".to_string());

        out.push(CronJobSummary {
            namespace,
            name,
            schedule,
            suspend,
            active,
            last_schedule,
            age,
        });
    }

    out.sort_by(|a, b| a.namespace.cmp(&b.namespace).then(a.name.cmp(&b.name)));
    out
}

fn format_duration(seconds: i64) -> String {
    if seconds < 60 {
        format!("{}s", seconds)
    } else if seconds < 3600 {
        format!("{}m", seconds / 60)
    } else if seconds < 86400 {
        format!("{}h", seconds / 3600)
    } else {
        format!("{}d", seconds / 86400)
    }
}

pub(super) async fn fetch_cronjob_details(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    namespace: &str,
    name: &str,
) -> anyhow::Result<()> {
    let client = state.cluster.client_or_err()?;
    let api: Api<CronJob> = Api::namespaced(client, namespace);
    let cronjob = api.get(name).await?;

    let spec = cronjob.spec.as_ref();
    let status = cronjob.status.as_ref();

    let schedule = spec
        .map(|s| s.schedule.clone())
        .unwrap_or_else(|| "-".to_string());
    let suspend = spec.and_then(|s| s.suspend).unwrap_or(false);
    let active = status
        .and_then(|s| s.active.as_ref())
        .map(|a| a.len() as i32)
        .unwrap_or(0);

    let last_schedule_time = status
        .and_then(|s| s.last_schedule_time.as_ref())
        .map(|ts| ts.0.to_rfc3339());

    let concurrency_policy = spec
        .and_then(|s| s.concurrency_policy.as_ref())
        .cloned()
        .unwrap_or_else(|| "Allow".to_string());

    let starting_deadline_seconds = spec.and_then(|s| s.starting_deadline_seconds);
    let successful_jobs_history_limit = spec.and_then(|s| s.successful_jobs_history_limit);
    let failed_jobs_history_limit = spec.and_then(|s| s.failed_jobs_history_limit);

    let labels = cronjob
        .metadata
        .labels
        .as_ref()
        .map(|m| {
            let mut pairs: Vec<_> = m.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
            pairs.sort_by(|a, b| a.0.cmp(&b.0));
            pairs
        })
        .unwrap_or_default();

    let annotations = cronjob
        .metadata
        .annotations
        .as_ref()
        .map(|m| {
            let mut pairs: Vec<_> = m.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
            pairs.sort_by(|a, b| a.0.cmp(&b.0));
            pairs
        })
        .unwrap_or_default();

    let details = CronJobDetails {
        namespace: namespace.to_string(),
        name: name.to_string(),
        schedule,
        suspend,
        active,
        last_schedule_time,
        concurrency_policy,
        starting_deadline_seconds,
        successful_jobs_history_limit,
        failed_jobs_history_limit,
        labels,
        annotations,
    };

    events::emit(
        state.cluster.seq.as_ref(),
        sender,
        Route::namespaced(
            section_keys::workloads_cronjobs(),
            state.cluster.current_context.clone(),
            Some(namespace.to_string()),
        ),
        Correlation::None,
        BackendEvent::ResourceRead(ResourceReadEvent::CronJobDetailsUpdated(details)),
    );
    Ok(())
}

pub(super) async fn delete_cronjob(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    namespace: &str,
    name: &str,
) -> anyhow::Result<()> {
    let client = state.cluster.client_or_err()?;
    let api: Api<CronJob> = Api::namespaced(client, namespace);

    let ts = Utc::now().to_rfc3339();
    let _ = interactions::annotate_action(
        &api,
        name,
        state.cluster.current_context.as_deref(),
        "delete",
        &ts,
        json!({}),
    )
    .await;

    api.delete(name, &DeleteParams::default()).await?;
    events::notify_info(
        state.cluster.seq.as_ref(),
        sender,
        format!("Deleted cronjob {}/{}", namespace, name),
    );
    Ok(())
}

pub(super) async fn suspend_cronjob(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    namespace: &str,
    name: &str,
    suspend: bool,
) -> anyhow::Result<()> {
    let client = state.cluster.client_or_err()?;
    let api: Api<CronJob> = Api::namespaced(client, namespace);

    let ts = Utc::now().to_rfc3339();
    let action = if suspend { "suspend" } else { "resume" };
    let _ = interactions::annotate_action(
        &api,
        name,
        state.cluster.current_context.as_deref(),
        action,
        &ts,
        json!({"suspend": suspend}),
    )
    .await;

    let patch = json!({
        "spec": {
            "suspend": suspend
        }
    });
    api.patch(name, &PatchParams::default(), &Patch::Merge(&patch))
        .await?;

    let msg = if suspend {
        format!("Suspended cronjob {}/{}", namespace, name)
    } else {
        format!("Resumed cronjob {}/{}", namespace, name)
    };
    events::notify_info(state.cluster.seq.as_ref(), sender, msg);
    Ok(())
}

pub(super) async fn trigger_cronjob(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    namespace: &str,
    name: &str,
) -> anyhow::Result<()> {
    use k8s_openapi::api::batch::v1::Job;
    let client = state.cluster.client_or_err()?;
    let cj_api: Api<CronJob> = Api::namespaced(client.clone(), namespace);
    let job_api: Api<Job> = Api::namespaced(client, namespace);

    let ts = Utc::now().to_rfc3339();
    let _ = interactions::annotate_action(
        &cj_api,
        name,
        state.cluster.current_context.as_deref(),
        "trigger",
        &ts,
        json!({}),
    )
    .await;

    // Get the cronjob to clone its job template
    let cronjob = cj_api.get(name).await?;
    let job_template = cronjob
        .spec
        .as_ref()
        .map(|s| &s.job_template)
        .ok_or_else(|| anyhow::anyhow!("CronJob has no job template"))?;

    let job_name = format!("{}-manual-{}", name, Utc::now().timestamp());
    let mut job: Job = serde_json::from_value(serde_json::to_value(&job_template.spec)?)?;

    // Set metadata
    job.metadata.name = Some(job_name.clone());
    job.metadata.namespace = Some(namespace.to_string());
    if job.metadata.labels.is_none() {
        job.metadata.labels = Some(Default::default());
    }
    job.metadata
        .labels
        .as_mut()
        .unwrap()
        .insert("cronjob".to_string(), name.to_string());
    job.metadata
        .labels
        .as_mut()
        .unwrap()
        .insert("manual-trigger".to_string(), "true".to_string());

    job_api.create(&Default::default(), &job).await?;
    events::notify_info(
        state.cluster.seq.as_ref(),
        sender,
        format!(
            "Triggered cronjob {}/{} -> job {}",
            namespace, name, job_name
        ),
    );
    Ok(())
}
