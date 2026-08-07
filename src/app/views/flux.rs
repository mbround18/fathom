/// Flux CD dashboard: reconciliation health for Sources, Kustomizations, and
/// HelmReleases, discovered dynamically from whatever Flux CRDs are installed.
use eframe::egui;
use egui::{Color32, RichText, ScrollArea, Ui};
use egui_plot::{Bar, BarChart, Legend, Plot};

use super::traits::{ViewAction, ViewComponent};
use crate::protocol::{
    BackendCmd, BackendEvent, FluxOverview, FluxReadyState, FluxResourceSummary, SystemEvent,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FluxTab {
    Sources,
    Kustomizations,
    HelmReleases,
}

impl FluxTab {
    fn label(self) -> &'static str {
        match self {
            FluxTab::Sources => "Sources",
            FluxTab::Kustomizations => "Kustomizations",
            FluxTab::HelmReleases => "HelmReleases",
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct FluxState {
    pub overview: FluxOverview,
    pub last_updated: Option<String>,
    pub scanning: bool,
}

pub struct FluxView {
    state: FluxState,
    tab: FluxTab,
    /// (kind, namespace, name) of the selected resource, shown in the details panel.
    selected: Option<(String, String, String)>,
}

impl FluxView {
    pub fn new() -> Self {
        Self {
            state: FluxState::default(),
            tab: FluxTab::Sources,
            selected: None,
        }
    }

    fn active_items(&self) -> &[FluxResourceSummary] {
        match self.tab {
            FluxTab::Sources => &self.state.overview.sources,
            FluxTab::Kustomizations => &self.state.overview.kustomizations,
            FluxTab::HelmReleases => &self.state.overview.helmreleases,
        }
    }

    fn selected_item(&self) -> Option<&FluxResourceSummary> {
        let (kind, namespace, name) = self.selected.as_ref()?;
        [
            &self.state.overview.sources,
            &self.state.overview.kustomizations,
            &self.state.overview.helmreleases,
        ]
        .into_iter()
        .flatten()
        .find(|it| &it.kind == kind && &it.namespace == namespace && &it.name == name)
    }

    fn render_summary_card(&mut self, ui: &mut Ui, tab: FluxTab, items: &[FluxResourceSummary]) {
        let (ready, other, failed) = counts_by_bucket(items);
        let is_active = self.tab == tab;

        let mut frame = egui::Frame::group(ui.style());
        if is_active {
            frame = frame.fill(ui.visuals().selection.bg_fill.linear_multiply(0.15));
        }

        let response = frame
            .show(ui, |ui| {
                ui.set_min_width(150.0);
                ui.vertical(|ui| {
                    ui.label(RichText::new(tab.label()).strong());
                    ui.label(RichText::new(items.len().to_string()).size(22.0).strong());
                    ui.horizontal(|ui| {
                        ui.colored_label(status_color(FluxReadyState::Ready), format!("✓ {ready}"));
                        if other > 0 {
                            ui.colored_label(
                                status_color(FluxReadyState::Reconciling),
                                format!("◔ {other}"),
                            );
                        }
                        if failed > 0 {
                            ui.colored_label(
                                status_color(FluxReadyState::Failed),
                                format!("✗ {failed}"),
                            );
                        }
                    });
                });
            })
            .response
            .interact(egui::Sense::click());

        if response.clicked() {
            self.tab = tab;
        }
    }

    fn render_status_chart(&self, ui: &mut Ui) {
        let categories: [(&str, &[FluxResourceSummary]); 3] = [
            ("Sources", &self.state.overview.sources),
            ("Kustomizations", &self.state.overview.kustomizations),
            ("HelmReleases", &self.state.overview.helmreleases),
        ];

        let states = [
            FluxReadyState::Ready,
            FluxReadyState::Reconciling,
            FluxReadyState::Failed,
            FluxReadyState::Suspended,
            FluxReadyState::Unknown,
        ];

        let mut running = [0.0_f64; 3];
        let mut series: Vec<BarChart> = Vec::new();
        for state in states {
            let mut bars = Vec::new();
            for (i, (_, items)) in categories.iter().enumerate() {
                let count = items.iter().filter(|it| it.ready == state).count() as f64;
                if count > 0.0 {
                    bars.push(
                        Bar::new(i as f64, count)
                            .base_offset(running[i])
                            .fill(status_color(state))
                            .width(0.6)
                            .name(state.label()),
                    );
                }
                running[i] += count;
            }
            if !bars.is_empty() {
                series.push(BarChart::new(state.label(), bars).color(status_color(state)));
            }
        }

        Plot::new("flux_status_chart")
            .height(160.0)
            .legend(Legend::default())
            .allow_zoom(false)
            .allow_scroll(false)
            .allow_drag(false)
            .allow_boxed_zoom(false)
            .show_axes([false, true])
            .show_grid([false, true])
            .x_axis_formatter(|mark, _range| match mark.value.round() as i64 {
                0 => "Sources".to_owned(),
                1 => "Kustomizations".to_owned(),
                2 => "HelmReleases".to_owned(),
                _ => String::new(),
            })
            .show(ui, |plot_ui| {
                for chart in series {
                    plot_ui.bar_chart(chart);
                }
            });
    }

    fn render_list(&mut self, ui: &mut Ui) -> Vec<ViewAction> {
        let actions = Vec::new();
        let items = self.active_items().to_vec();

        if items.is_empty() {
            ui.label(
                RichText::new(format!("No {} found.", self.tab.label().to_lowercase())).weak(),
            );
            return actions;
        }

        ScrollArea::vertical()
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                egui::Grid::new("flux_grid")
                    .striped(true)
                    .min_col_width(80.0)
                    .show(ui, |ui| {
                        ui.label(RichText::new("").strong());
                        ui.label(RichText::new("Name").strong());
                        ui.label(RichText::new("Namespace").strong());
                        ui.label(RichText::new("Revision").strong());
                        ui.label(RichText::new("Message").strong());
                        ui.end_row();

                        for item in &items {
                            ui.colored_label(status_color(item.ready), status_glyph(item.ready));

                            let is_selected = self.selected.as_ref().is_some_and(|(k, ns, n)| {
                                k == &item.kind && ns == &item.namespace && n == &item.name
                            });
                            if ui.selectable_label(is_selected, &item.name).clicked() {
                                self.selected = Some((
                                    item.kind.clone(),
                                    item.namespace.clone(),
                                    item.name.clone(),
                                ));
                            }

                            ui.label(&item.namespace);
                            ui.label(
                                RichText::new(short_revision(&item.revision))
                                    .monospace()
                                    .weak(),
                            );

                            let message = if item.suspended {
                                "Suspended".to_owned()
                            } else if item.message.is_empty() {
                                "-".to_owned()
                            } else {
                                item.message.clone()
                            };
                            ui.label(RichText::new(truncate(&message, 60)).weak());
                            ui.end_row();
                        }
                    });
            });

        actions
    }
}

impl Default for FluxView {
    fn default() -> Self {
        Self::new()
    }
}

impl ViewComponent for FluxView {
    fn id(&self) -> &'static str {
        "integrations_flux"
    }

    fn render(&mut self, ui: &mut Ui, _ctx: &egui::Context) -> Vec<ViewAction> {
        let mut actions = Vec::new();

        ui.horizontal(|ui| {
            ui.heading("Flux CD");
            let label = if self.state.scanning {
                "Scanning..."
            } else {
                "⟳ Refresh"
            };
            if ui
                .add_enabled(!self.state.scanning, egui::Button::new(label))
                .clicked()
            {
                self.state.scanning = true;
                actions.push(ViewAction::SendCommand(BackendCmd::RefreshFlux));
            }
            if let Some(updated) = &self.state.last_updated {
                ui.label(
                    RichText::new(format!("Last updated: {updated}"))
                        .weak()
                        .size(10.0),
                );
            }
        });
        ui.separator();

        if !self.state.overview.installed {
            ui.add_space(24.0);
            ui.vertical_centered(|ui| {
                ui.label(RichText::new("Flux not detected").size(16.0).strong());
                ui.label(
                    RichText::new("No Flux GitOps Toolkit CRDs (source/kustomize/helm.toolkit.fluxcd.io) found in this cluster.")
                        .weak(),
                );
            });
            return actions;
        }

        ui.horizontal(|ui| {
            self.render_summary_card(ui, FluxTab::Sources, &self.state.overview.sources.clone());
            self.render_summary_card(
                ui,
                FluxTab::Kustomizations,
                &self.state.overview.kustomizations.clone(),
            );
            self.render_summary_card(
                ui,
                FluxTab::HelmReleases,
                &self.state.overview.helmreleases.clone(),
            );
        });

        ui.add_space(12.0);
        self.render_status_chart(ui);
        ui.add_space(12.0);
        ui.separator();

        ui.horizontal(|ui| {
            for tab in [
                FluxTab::Sources,
                FluxTab::Kustomizations,
                FluxTab::HelmReleases,
            ] {
                if ui.selectable_label(self.tab == tab, tab.label()).clicked() {
                    self.tab = tab;
                }
            }
        });
        ui.add_space(6.0);

        let list_actions = self.render_list(ui);
        actions.extend(list_actions);

        actions
    }

    fn on_event(&mut self, event: &BackendEvent) {
        if let BackendEvent::System(SystemEvent::FluxOverviewUpdated(overview)) = event {
            self.state.overview = overview.clone();
            self.state.scanning = false;
            self.state.last_updated = Some(chrono::Local::now().format("%H:%M:%S").to_string());
        }
    }

    fn on_mount(&mut self) -> Vec<ViewAction> {
        self.state.scanning = true;
        vec![ViewAction::SendCommand(BackendCmd::RefreshFlux)]
    }

    fn has_details(&self) -> bool {
        self.selected.is_some()
    }

    fn render_details(&mut self, ui: &mut Ui, _ctx: &egui::Context) -> (bool, Vec<ViewAction>) {
        let Some(item) = self.selected_item().cloned() else {
            return (false, Vec::new());
        };

        ui.heading(&item.kind);
        ui.separator();

        ui.label(RichText::new(&item.name).monospace().strong());
        ui.label(RichText::new(format!("Namespace: {}", item.namespace)).weak());
        ui.add_space(8.0);

        ui.horizontal(|ui| {
            ui.label("Status:");
            ui.colored_label(status_color(item.ready), item.ready.label());
        });

        if !item.revision.is_empty() {
            ui.label(format!("Revision: {}", item.revision));
        }
        if !item.interval.is_empty() {
            ui.label(format!("Interval: {}", item.interval));
        }
        if let Some(source_ref) = &item.source_ref {
            ui.label(format!("Source: {source_ref}"));
        }
        if item.suspended {
            ui.colored_label(Color32::GRAY, "⏸ Suspended");
        }
        if !item.last_transition.is_empty() {
            ui.label(
                RichText::new(format!("Last transition: {}", item.last_transition))
                    .weak()
                    .size(10.0),
            );
        }

        if !item.message.is_empty() {
            ui.add_space(8.0);
            ui.group(|ui| {
                ui.label(RichText::new("Message").strong());
                ui.separator();
                ui.label(&item.message);
            });
        }

        (true, Vec::new())
    }
}

fn status_color(state: FluxReadyState) -> Color32 {
    match state {
        FluxReadyState::Ready => Color32::from_rgb(76, 175, 80),
        FluxReadyState::Reconciling => Color32::from_rgb(255, 193, 7),
        FluxReadyState::Failed => Color32::from_rgb(244, 67, 54),
        FluxReadyState::Suspended => Color32::GRAY,
        FluxReadyState::Unknown => Color32::DARK_GRAY,
    }
}

fn status_glyph(state: FluxReadyState) -> &'static str {
    match state {
        FluxReadyState::Ready => "●",
        FluxReadyState::Reconciling => "◔",
        FluxReadyState::Failed => "✗",
        FluxReadyState::Suspended => "⏸",
        FluxReadyState::Unknown => "?",
    }
}

/// (ready, in-progress-ish [reconciling/suspended/unknown], failed) for card summaries.
fn counts_by_bucket(items: &[FluxResourceSummary]) -> (usize, usize, usize) {
    let mut ready = 0;
    let mut other = 0;
    let mut failed = 0;
    for item in items {
        match item.ready {
            FluxReadyState::Ready => ready += 1,
            FluxReadyState::Failed => failed += 1,
            _ => other += 1,
        }
    }
    (ready, other, failed)
}

fn short_revision(revision: &str) -> String {
    // Flux revisions look like "main@sha1:abcdef123..." or "1.2.3+abcdef12" - keep it short.
    if let Some((_, sha)) = revision.rsplit_once(':') {
        truncate(sha, 12)
    } else {
        truncate(revision, 16)
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_owned()
    } else {
        let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}
