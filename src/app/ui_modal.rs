use super::*;

impl FathomApp {
    pub(super) fn render_modal(&mut self, ctx: &egui::Context) {
        // get_or_create_view (not just get_mut) - render_modal runs every
        // frame, so this guarantees ModalView exists from frame 1 onward.
        // Without it, `open_confirm`/`open_scale_workload`/`open_bulk_delete`
        // (which only `get_mut`, never create) silently no-op on a fresh
        // session until something else happens to create the view first -
        // e.g. delete confirmations did nothing at all unless a port-forward
        // modal had been opened earlier in the same session.
        let view = self.get_or_create_view(&section_keys::modal());
        let ptr = view as *mut dyn ViewComponent as *mut ModalView;
        if let Some(modal_view) = unsafe { ptr.as_mut() } {
            // Call render_modal_window which handles egui::Window rendering
            let actions = modal_view.render_modal_window(ctx);

            // Process returned actions
            for action in actions {
                match action {
                    ViewAction::SendCommand(cmd) => {
                        self.send(cmd);
                    }
                    ViewAction::NotifyStatus(msg) => {
                        self.push_status(msg);
                    }
                    ViewAction::Navigate(_) => {
                        // Modal shouldn't emit navigation
                    }
                    ViewAction::OpenModal(_) => {
                        // Modal shouldn't emit nested modals
                    }
                    ViewAction::OpenScaleModal(_, _, _, _) => {
                        // Modal shouldn't emit nested modals
                    }
                    ViewAction::OpenUrl(url) => {
                        if let Err(e) = open::that(&url) {
                            tracing::warn!("Failed to open URL {}: {}", url, e);
                            self.push_status(format!("Failed to open URL: {}", e));
                        }
                    }
                }
            }
        }
    }
}
