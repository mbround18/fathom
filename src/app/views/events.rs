/// Events section: displays recent Kubernetes events with filtering.
use egui::{RichText, ScrollArea, Ui};

use super::traits::{ViewAction, ViewComponent};
use crate::protocol::{BackendCmd, BackendEvent, KubernetesEvent};

#[derive(Debug, Clone)]
pub struct EventsState {
    pub events: Vec<KubernetesEvent>,
    pub filter_namespace: Option<String>,
    pub filter_type: Option<String>,
}

impl Default for EventsState {
    fn default() -> Self {
        Self {
            events: Vec::new(),
            filter_namespace: None,
            filter_type: None,
        }
    }
}

pub struct EventsView {
    pub state: EventsState,
}

impl EventsView {
    pub fn new() -> Self {
        Self {
            state: EventsState::default(),
        }
    }
}

impl Default for EventsView {
    fn default() -> Self {
        Self::new()
    }
}

impl ViewComponent for EventsView {
    fn id(&self) -> &'static str {
        "events"
    }

    fn render(&mut self, ui: &mut Ui, _ctx: &egui::Context) -> Vec<ViewAction> {
        ui.heading("Recent Events");
        ui.separator();

        // Filters
        ui.horizontal(|ui| {
            ui.label("Namespace:");
            let mut ns_filter = self.state.filter_namespace.clone().unwrap_or_default();
            if ui.text_edit_singleline(&mut ns_filter).changed() {
                self.state.filter_namespace = if ns_filter.is_empty() {
                    None
                } else {
                    Some(ns_filter)
                };
            }

            ui.label("Type:");
            let mut type_filter = self.state.filter_type.clone().unwrap_or_default();
            if ui.text_edit_singleline(&mut type_filter).changed() {
                self.state.filter_type = if type_filter.is_empty() {
                    None
                } else {
                    Some(type_filter)
                };
            }
        });

        ui.separator();

        // Events list
        let filtered_events: Vec<_> = self
            .state
            .events
            .iter()
            .filter(|e| {
                if let Some(ns) = &self.state.filter_namespace {
                    if !e.namespace.contains(ns) {
                        return false;
                    }
                }
                if let Some(typ) = &self.state.filter_type {
                    if !e.event_type.contains(typ) {
                        return false;
                    }
                }
                true
            })
            .collect();

        if filtered_events.is_empty() {
            ui.label(RichText::new("No events").weak());
        } else {
            ScrollArea::vertical()
                .auto_shrink([false; 2])
                .show(ui, |ui| {
                    egui::Grid::new("events_grid")
                        .striped(true)
                        .min_col_width(80.0)
                        .show(ui, |ui| {
                            // Headers
                            ui.label(RichText::new("Type").strong());
                            ui.label(RichText::new("Object").strong());
                            ui.label(RichText::new("Namespace").strong());
                            ui.label(RichText::new("Count").strong());
                            ui.label(RichText::new("First Timestamp").strong());
                            ui.label(RichText::new("Last Timestamp").strong());
                            ui.label(RichText::new("Message").strong());
                            ui.end_row();

                            // Rows
                            for event in filtered_events {
                                ui.label(&event.event_type);
                                ui.label(&event.involved_object_kind);
                                ui.label(&event.namespace);
                                ui.label(event.count.to_string());
                                ui.label(&event.first_timestamp);
                                ui.label(&event.last_timestamp);
                                ui.label(&event.message);
                                ui.end_row();
                            }
                        });
                });
        }

        Vec::new()
    }

    fn on_event(&mut self, event: &BackendEvent) {
        use crate::protocol::ResourceListEvent;
        if let BackendEvent::ResourceList(ResourceListEvent::EventsUpdated(summary)) = event {
            self.state.events = summary.events.clone();
        }
    }

    fn on_mount(&mut self) -> Vec<ViewAction> {
        vec![ViewAction::SendCommand(BackendCmd::RefreshEvents)]
    }
}
