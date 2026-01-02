pub mod context;
pub mod cronjobs;
pub mod integrations;
pub mod jobs;
pub mod mcp_integration;
pub mod modal;
pub mod navigation;
pub mod placeholder;
pub mod pods;
pub mod registry;
pub mod status;
pub mod traits;
pub mod workloads;

pub use context::ContextView;
pub use cronjobs::CronJobsView;
pub use integrations::IntegrationsView;
pub use jobs::JobsView;
pub use mcp_integration::McpIntegrationView;
pub use modal::{ModalState, ModalView};
pub use navigation::NavigationView;
pub use placeholder::PlaceholderView;
pub use pods::{PodTab, PodsSort, PodsSortKey, PodsView, SortDir};
pub use registry::ViewRegistry;
pub use status::StatusView;
pub use traits::{BoxedView, ViewAction, ViewComponent};
pub use workloads::{
    DaemonSetView, DeploymentView, ReplicaSetView, ReplicationControllerView, StatefulSetView,
    WorkloadViewState,
};
