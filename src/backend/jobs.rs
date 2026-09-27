use crate::protocol::{
    BackendEvent, BackendEventEnvelope, Correlation, JobCondition, JobDetails, JobSummary,
    ResourceListEvent, ResourceReadEvent, Route, section_keys,
};
use crossbeam_channel::Sender;
use k8s_openapi::{api::batch::v1::Job, chrono::Utc};
use kube::ResourceExt;
use kube::api::{Api, DeleteParams};
use serde_json::json;

use super::interactions;
use super::{events, state::BackendState};

pub(super) async fn refresh_jobs(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
) -> anyhow::Result<()> {
    let jobs = state.cluster.list_namespaced::<Job>().await?;
    let summaries = jobs_to_summaries(jobs, &state.cluster.namespace_filter);

    events::emit(
        state.cluster.seq.as_ref(),
        sender,
        Route::namespaced(
            section_keys::workloads_jobs(),
            state.cluster.current_context.clone(),
            None,
        ),
        Correlation::None,
        BackendEvent::ResourceList(ResourceListEvent::JobsUpdated(summaries)),
    );
    Ok(())
}

fn jobs_to_summaries(jobs: Vec<Job>, filter: &[String]) -> Vec<JobSummary> {
    let mut out: Vec<JobSummary> = Vec::new();

    for job in jobs {
        let namespace = job.namespace().unwrap_or_default();
        if !filter.is_empty() && !filter.iter().any(|ns| ns == &namespace) {
            continue;
        }

        let name = job.name_any();
        let status = job.status.as_ref();
        let active = status.and_then(|s| s.active).unwrap_or(0);
        let succeeded = status.and_then(|s| s.succeeded).unwrap_or(0);
        let failed = status.and_then(|s| s.failed).unwrap_or(0);

        let completions_target = job.spec.as_ref().and_then(|s| s.completions).unwrap_or(1);

        let completions = if succeeded >= completions_target {
            "Completed".to_string()
        } else {
            format!("{}/{}", succeeded, completions_target)
        };

        let start_time = status.and_then(|s| s.start_time.as_ref());
        let completion_time = status.and_then(|s| s.completion_time.as_ref());

        let duration = match (start_time, completion_time) {
            (Some(start), Some(end)) => {
                let dur = end.0.signed_duration_since(start.0);
                format_duration(dur.num_seconds())
            }
            (Some(start), None) => {
                let now = Utc::now();
                let dur = now.signed_duration_since(start.0);
                format_duration(dur.num_seconds())
            }
            _ => "-".to_string(),
        };

        let age = job
            .metadata
            .creation_timestamp
            .as_ref()
            .map(|ts| {
                let now = Utc::now();
                let dur = now.signed_duration_since(ts.0);
                format_duration(dur.num_seconds())
            })
            .unwrap_or_else(|| "-".to_string());

        out.push(JobSummary {
            namespace,
            name,
            completions,
            duration,
            active,
            succeeded,
            failed,
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
        format!("{}m{}s", seconds / 60, seconds % 60)
    } else if seconds < 86400 {
        format!("{}h{}m", seconds / 3600, (seconds % 3600) / 60)
    } else {
        format!("{}d{}h", seconds / 86400, (seconds % 86400) / 3600)
    }
}

pub(super) async fn fetch_job_details(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    namespace: &str,
    name: &str,
) -> anyhow::Result<()> {
    let client = state.cluster.client_or_err()?;
    let api: Api<Job> = Api::namespaced(client, namespace);
    let job = api.get(name).await?;

    let status = job.status.as_ref();
    let active = status.and_then(|s| s.active).unwrap_or(0);
    let succeeded = status.and_then(|s| s.succeeded).unwrap_or(0);
    let failed = status.and_then(|s| s.failed).unwrap_or(0);

    let completions = job.spec.as_ref().and_then(|s| s.completions);
    let parallelism = job.spec.as_ref().and_then(|s| s.parallelism);

    let start_time = status
        .and_then(|s| s.start_time.as_ref())
        .map(|ts| ts.0.to_rfc3339());
    let completion_time = status
        .and_then(|s| s.completion_time.as_ref())
        .map(|ts| ts.0.to_rfc3339());

    let duration = job_duration(status);

    let selector = job
        .spec
        .as_ref()
        .and_then(|s| s.selector.as_ref())
        .and_then(|sel| sel.match_labels.as_ref())
        .map(|labels| {
            labels
                .iter()
                .map(|(k, v)| format!("{}={}", k, v))
                .collect::<Vec<_>>()
                .join(",")
        })
        .unwrap_or_else(|| "-".to_string());

    let labels = sorted_pairs(job.metadata.labels.as_ref());
    let annotations = sorted_pairs(job.metadata.annotations.as_ref());

    let conditions = job_conditions(status);

    let details = JobDetails {
        namespace: namespace.to_string(),
        name: name.to_string(),
        active,
        succeeded,
        failed,
        completions,
        parallelism,
        start_time,
        completion_time,
        duration,
        selector,
        labels,
        annotations,
        conditions,
    };

    events::emit(
        state.cluster.seq.as_ref(),
        sender,
        Route::namespaced(
            section_keys::workloads_jobs(),
            state.cluster.current_context.clone(),
            Some(namespace.to_string()),
        ),
        Correlation::None,
        BackendEvent::ResourceRead(ResourceReadEvent::JobDetailsUpdated(details)),
    );
    Ok(())
}

pub(super) async fn delete_job(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    namespace: &str,
    name: &str,
) -> anyhow::Result<()> {
    let client = state.cluster.client_or_err()?;
    let api: Api<Job> = Api::namespaced(client, namespace);

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
        format!("Deleted job {}/{}", namespace, name),
    );
    Ok(())
}

/// Elapsed time from start to completion (or now, if still running); `-` if never started.
fn job_duration(status: Option<&k8s_openapi::api::batch::v1::JobStatus>) -> String {
    match (
        status.and_then(|s| s.start_time.as_ref()),
        status.and_then(|s| s.completion_time.as_ref()),
    ) {
        (Some(start), Some(end)) => {
            let dur = end.0.signed_duration_since(start.0);
            format_duration(dur.num_seconds())
        }
        (Some(start), None) => {
            let now = Utc::now();
            let dur = now.signed_duration_since(start.0);
            format_duration(dur.num_seconds())
        }
        _ => "-".to_string(),
    }
}

/// Key/value pairs sorted by key.
fn sorted_pairs(map: Option<&std::collections::BTreeMap<String, String>>) -> Vec<(String, String)> {
    map.map(|m| {
        let mut pairs: Vec<_> = m.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        pairs.sort_by(|a, b| a.0.cmp(&b.0));
        pairs
    })
    .unwrap_or_default()
}

fn job_conditions(status: Option<&k8s_openapi::api::batch::v1::JobStatus>) -> Vec<JobCondition> {
    status
        .and_then(|s| s.conditions.as_ref())
        .map(|conds| {
            conds
                .iter()
                .map(|c| JobCondition {
                    condition_type: c.type_.clone(),
                    status: c.status.clone(),
                    reason: c.reason.clone(),
                    message: c.message.clone(),
                    last_transition: c.last_transition_time.as_ref().map(|ts| ts.0.to_rfc3339()),
                })
                .collect()
        })
        .unwrap_or_default()
}
