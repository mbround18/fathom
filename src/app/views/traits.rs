use crate::app::{ConfirmAction, WorkloadKind};
use crate::protocol::{BackendEvent, SectionKey};

/// UI intent/action emitted by views to communicate with backend
#[derive(Clone, Debug)]
pub enum ViewAction {
    /// Request to send a command to the backend
    SendCommand(crate::protocol::BackendCmd),
    /// Update status log with a message
    NotifyStatus(String),
    /// Navigate to a different section
    Navigate(SectionKey),
    /// Open a confirmation modal
    OpenModal(ConfirmAction),
    /// Open a scaling modal for a workload (namespace, name, current replicas, kind)
    OpenScaleModal(WorkloadKind, String, String, i32),
    /// Open a URL in the default browser
    OpenUrl(String),
}

/// Core trait for pluggable view components
///
/// A view handles rendering a specific section of the UI and emits actions
/// for the shell/coordinator to translate into backend commands.
pub trait ViewComponent: Send {
    /// Unique identifier for this view (e.g., "workloads_pods", "integrations_prometheus")
    fn id(&self) -> &'static str;

    /// Render the view in the provided UI context
    ///
    /// # Arguments
    /// * `ui` - egui context for rendering
    /// * `ctx` - eframe context for frame control (repaints, etc.)
    ///
    /// # Returns
    /// Vector of actions emitted by user interactions this frame
    fn render(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) -> Vec<ViewAction>;

    /// Render the details panel (right side) for this view
    ///
    /// Called by FathomApp's render_right_panel() to display details for selected resource.
    /// Return true if the view has something to show in the details panel (resource selected).
    /// Return false to show a placeholder message.
    ///
    /// # Arguments
    /// * `ui` - egui context for right panel rendering
    /// * `ctx` - eframe context for frame control
    ///
    /// # Returns
    /// (has_content, actions) - whether content was rendered and any actions emitted
    fn render_details(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
    ) -> (bool, Vec<ViewAction>) {
        // Default: no details panel
        let _ = (ui, ctx);
        (false, Vec::new())
    }

    /// Handle a backend event (e.g., resource list update, error notification)
    ///
    /// Views subscribe to relevant events by pattern-matching in this method.
    /// Events are routed after backend sends them.
    fn on_event(&mut self, event: &BackendEvent);

    /// Called when this view becomes active (switched to via navigation)
    ///
    /// Use this to send initial fetch commands (e.g., RefreshPods) or reset state.
    fn on_mount(&mut self) -> Vec<ViewAction> {
        Vec::new()
    }

    /// Called when this view is deselected (another view is switching in)
    ///
    /// Use this to stop background tasks or clear streaming state.
    fn on_unmount(&mut self) -> Vec<ViewAction> {
        Vec::new()
    }
}

/// Trait for views that can be boxed dynamically
pub type BoxedView = Box<dyn ViewComponent>;
