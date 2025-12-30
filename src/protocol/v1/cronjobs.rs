/// Protocol types for Kubernetes CronJobs (batch/v1)

#[derive(Debug, Clone)]
pub struct CronJobSummary {
    pub namespace: String,
    pub name: String,
    pub schedule: String,
    pub suspend: bool,
    pub active: i32,
    pub last_schedule: Option<String>,
    pub age: String,
}

#[derive(Debug, Clone)]
pub struct CronJobDetails {
    pub namespace: String,
    pub name: String,
    pub schedule: String,
    pub suspend: bool,
    pub active: i32,
    pub last_schedule_time: Option<String>,
    pub concurrency_policy: String,
    pub starting_deadline_seconds: Option<i64>,
    pub successful_jobs_history_limit: Option<i32>,
    pub failed_jobs_history_limit: Option<i32>,
    pub labels: Vec<(String, String)>,
    pub annotations: Vec<(String, String)>,
}
