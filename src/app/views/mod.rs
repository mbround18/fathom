pub mod pods;
pub mod registry;
pub mod traits;
pub mod workloads;

pub use pods::{PodTab, PodsSort, PodsSortKey, PodsView, SortDir};
pub use registry::ViewRegistry;
pub use traits::{BoxedView, ViewAction, ViewComponent};
pub use workloads::{
    DaemonSetView, DeploymentView, ReplicaSetView, ReplicationControllerView, StatefulSetView,
    WorkloadViewState,
};
