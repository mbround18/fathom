use super::*;

impl FathomApp {
    pub(super) fn render_modal(&mut self, ctx: &egui::Context) {
        let modal = self.modal.clone();
        match modal {
            ModalState::None => {}
            ModalState::Confirm {
                title,
                body,
                confirm_text,
                action,
            } => {
                let mut open = true;
                let mut should_close = false;
                egui::Window::new(title).open(&mut open).show(ctx, |ui| {
                    ui.label(&body);
                    ui.separator();
                    ui.horizontal(|ui| {
                        if ui.button("Cancel").clicked() {
                            should_close = true;
                        }
                        if ui.button(&confirm_text).clicked() {
                            match action {
                                ConfirmAction::DeletePod { namespace, name } => {
                                    self.send(BackendCmd::DeletePod { namespace, name });
                                }
                                ConfirmAction::DeleteDeployment { namespace, name } => {
                                    self.send(BackendCmd::DeleteDeployment { namespace, name });
                                }
                                ConfirmAction::RestartDeployment { namespace, name } => {
                                    self.send(BackendCmd::RestartDeployment { namespace, name });
                                }
                            }
                            should_close = true;
                        }
                    });
                });
                if !open || should_close {
                    self.modal = ModalState::None;
                }
            }
            ModalState::ScaleDeployment {
                namespace,
                name,
                current,
                desired,
            } => {
                let mut open = true;
                let mut should_close = false;
                let mut new_desired = desired;
                egui::Window::new("Scale Deployment")
                    .open(&mut open)
                    .show(ctx, |ui| {
                        ui.label(format!("{}/{}", namespace, name));
                        ui.label(format!("Current: {}", current));
                        ui.horizontal(|ui| {
                            ui.label("Desired:");
                            ui.add(egui::Slider::new(&mut new_desired, 0..=10).show_value(true));
                        });
                        ui.separator();
                        ui.horizontal(|ui| {
                            if ui.button("Cancel").clicked() {
                                should_close = true;
                            }
                            if ui.button("Scale").clicked() {
                                self.send(BackendCmd::ScaleDeployment {
                                    namespace,
                                    name,
                                    replicas: new_desired as i32,
                                });
                                should_close = true;
                            }
                        });
                    });
                if !open || should_close {
                    self.modal = ModalState::None;
                }
            }
        }
    }
}
