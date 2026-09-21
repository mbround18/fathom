//! Protocol types for Kubernetes Jobs (batch/v1)

use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct JobSummary {
    pub namespace: String,
    pub name: String,
    pub completions: String, // e.g., "5/10" or "Completed"
    pub duration: String,    // e.g., "2m30s"
    pub active: i32,
    pub succeeded: i32,
    pub failed: i32,
    pub age: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct JobDetails {
    pub namespace: String,
    pub name: String,
    pub active: i32,
    pub succeeded: i32,
    pub failed: i32,
    pub completions: Option<i32>,
    pub parallelism: Option<i32>,
    pub start_time: Option<String>,
    pub completion_time: Option<String>,
    pub duration: String,
    pub selector: String,
    pub labels: Vec<(String, String)>,
    pub annotations: Vec<(String, String)>,
    pub conditions: Vec<JobCondition>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct JobCondition {
    pub condition_type: String,
    pub status: String,
    pub reason: Option<String>,
    pub message: Option<String>,
    pub last_transition: Option<String>,
}
