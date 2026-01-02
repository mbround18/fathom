use super::traits::{ViewAction, ViewComponent};
use crate::protocol::{BackendEvent, SectionKey};
use eframe::egui;

/// Minimal placeholder view for sections that are not yet implemented.
pub struct PlaceholderView {
    title: String,
    section: SectionKey,
}

impl PlaceholderView {
    pub fn new(section: SectionKey) -> Self {
        let title = section.to_string();
        Self { title, section }
    }
}

impl ViewComponent for PlaceholderView {
    fn id(&self) -> &'static str {
        "placeholder"
    }

    fn render(&mut self, ui: &mut egui::Ui, _ctx: &egui::Context) -> Vec<ViewAction> {
        ui.heading(format!("{} (coming soon)", self.title));
        ui.add_space(8.0);
        ui.label("This section is defined but not implemented yet.");
        ui.label("Planned capabilities include metrics, details, and actions per MVP.");
        ui.monospace(format!("Section key: {}", self.section));
        Vec::new()
    }

    fn on_event(&mut self, _event: &BackendEvent) {}
}
