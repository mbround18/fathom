use super::*;

impl FathomApp {
    pub(super) fn render_modal(&mut self, ctx: &egui::Context) {
        // Get the modal view and render modals directly from ctx
        if let Some(view) = self.views.get_mut(&section_keys::modal().to_string()) {
            let ptr = view.as_mut() as *mut dyn ViewComponent as *mut ModalView;
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
                    }
                }
            }
        }
    }
}
