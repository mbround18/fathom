//! Shared rendering for the labels/annotations/raw-JSON blocks that appear at
//! the bottom of nearly every resource detail panel. Extracted because ~26
//! views had byte-identical copies of this code (one per k8s resource type).

/// Renders a collapsing section listing key/value pairs (e.g. labels or
/// annotations), or "(none)" if empty.
pub fn render_kv_collapsing<'a>(
    ui: &mut egui::Ui,
    title: &str,
    items: impl IntoIterator<Item = (&'a String, &'a String)>,
) {
    ui.collapsing(title, |ui| {
        let mut any = false;
        for (k, v) in items {
            ui.monospace(format!("{k}={v}"));
            any = true;
        }
        if !any {
            ui.label("(none)");
        }
    });
}

/// Renders the "Raw JSON" collapsing section with a scrollable monospace body.
pub fn render_raw_json_collapsing(ui: &mut egui::Ui, raw_json: &str) {
    ui.collapsing("Raw JSON", |ui| {
        egui::ScrollArea::vertical()
            .max_height(250.0)
            .show(ui, |ui| {
                ui.monospace(raw_json);
            });
    });
}
