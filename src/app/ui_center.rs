use super::*;

impl FathomApp {
    /// Render center panel using view component dispatch
    ///
    /// New view-based architecture:
    /// 1. Get or create view for active section
    /// 2. Render view in center panel
    /// 3. Process ViewActions returned by render
    pub(super) fn render_center(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().show(ctx, |ui| {
            // Prepare namespace filter before getting view (to avoid borrow conflicts)
            let filter = self.selected_namespaces.clone();

            // Get the view for the active section
            let key = self.active_section.clone();
            let view = self.get_or_create_view(&key);
            let view_id = view.id().to_string();

            // Drop the mutable borrow from get_or_create_view before calling apply_namespace_filter_to_view
            FathomApp::apply_namespace_filter_to_view(view, &view_id, &filter);

            // Render view and collect actions
            let actions = view.render(ui, ctx);

            // Process returned actions
            for action in actions {
                match action {
                    ViewAction::SendCommand(cmd) => {
                        // When a selection command is sent, show the details panel
                        match &cmd {
                            BackendCmd::SelectPod { .. }
                            | BackendCmd::SelectDeployment { .. }
                            | BackendCmd::SelectDaemonSet { .. }
                            | BackendCmd::SelectStatefulSet { .. }
                            | BackendCmd::SelectReplicaSet { .. }
                            | BackendCmd::SelectReplicationController { .. }
                            | BackendCmd::SelectJob { .. }
                            | BackendCmd::SelectCronJob { .. } => {
                                self.show_details_panel = true;
                            }
                            _ => {}
                        }
                        self.send(cmd);
                    }
                    ViewAction::NotifyStatus(msg) => {
                        self.push_status(msg);
                    }
                    ViewAction::Navigate(section) => {
                        self.active_section = section;
                    }
                    ViewAction::OpenModal(confirm_action) => {
                        // Bulk-delete from the center list view opens the BulkDelete modal directly
                        if let ConfirmAction::BulkDelete(items) = confirm_action {
                            self.open_bulk_delete(items);
                        }
                        // Other modal types from center panel are handled by the right panel
                    }
                    ViewAction::OpenScaleModal(_, _, _, _) => {
                        // Center view scale modals handled in right panel
                    }
                    ViewAction::OpenUrl(url) => {
                        if let Err(e) = open::that(&url) {
                            tracing::warn!("Failed to open URL {}: {}", url, e);
                            self.push_status(format!("Failed to open URL: {}", e));
                        }
                    }
                }
            }
        });
    }
}
