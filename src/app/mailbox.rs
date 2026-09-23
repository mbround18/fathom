use super::*;
use crate::protocol::{ResourceListEvent, StreamEvent};

impl FathomApp {
    #[allow(clippy::too_many_lines)] // removed by T125 (egui cutover)
    pub(super) fn check_mailbox(&mut self) {
        // First, handle view switching and lifecycle
        self.handle_view_switch();

        // Collect events first
        let mut events = Vec::new();
        while let Ok(envelope) = self.gui_rx.try_recv() {
            events.push(envelope);
        }

        // Now process events
        for envelope in events {
            // Request-completion signals are for the React bridge; the egui app sends no
            // request ids and reports failures through the Error notifications instead.
            if matches!(envelope.event, BackendEvent::CommandFinished { .. }) {
                continue;
            }
            let event = &envelope.event;

            // Route event to context view (always needs updates) - ensure it exists first
            let context_section = section_keys::context();
            let context_view = self.get_or_create_view(&context_section);
            context_view.on_event(event);

            // For ResourceListEvents, broadcast to ALL views since multiple views may care about the same data
            if matches!(&envelope.event, BackendEvent::ResourceList(_)) {
                // Broadcast to all instantiated views
                for view in self.views.values_mut() {
                    view.on_event(event);
                }
            } else {
                // For other events, route to active view only
                let active_section = self.active_section.clone();
                let active_view = self.get_or_create_view(&active_section);
                active_view.on_event(event);
            }

            match &envelope.event {
                BackendEvent::System(ev) => match ev {
                    SystemEvent::BackendStarted => {
                        self.push_status("Backend started.".to_owned());
                    }
                    SystemEvent::Notification { level, message } => match level {
                        NotificationLevel::Info => self.push_status(message.clone()),
                        NotificationLevel::Warn => self.push_status(format!("WARN: {}", message)),
                        NotificationLevel::Error => self.push_status(format!("ERROR: {}", message)),
                    },
                    SystemEvent::ContextsUpdated { .. } => {
                        // ContextView handles this in on_event
                    }
                    SystemEvent::ContextChanged { .. } => {
                        self.reset_views_for_context_switch();
                    }
                    SystemEvent::ContextStatusUpdated { .. } => {
                        // 012-incluster-daemon-federation P1: the Tauri/React UI owns this
                        // surface; the legacy egui shell doesn't render a context status list.
                    }
                    SystemEvent::NamespaceFilterChanged { .. } => {
                        // ContextView handles this in on_event
                    }
                    SystemEvent::PrometheusConnectionTest { success, message } => {
                        self.prometheus_testing = false;
                        if *success {
                            self.prometheus_connected = true;
                            self.push_status("Prometheus connected successfully".to_owned());
                        } else {
                            self.prometheus_connected = false;
                            self.push_status(format!("Prometheus connection failed: {}", message));
                        }
                    }
                    SystemEvent::GrafanaConnectionTest { success, message } => {
                        self.grafana_testing = false;
                        if *success {
                            self.grafana_connected = true;
                            self.push_status("Grafana connected successfully".to_owned());
                        } else {
                            self.grafana_connected = false;
                            self.push_status(format!("Grafana connection failed: {}", message));
                        }
                    }
                    SystemEvent::McpServerStatus {
                        running: _,
                        message: _,
                    } => {
                        // McpIntegrationView handles this in on_event
                    }
                    SystemEvent::PortAvailability {
                        port: _,
                        available: _,
                    } => {
                        // Port availability check result - modal handles this via on_event
                    }
                    SystemEvent::DiskUsageUpdated(items) => {
                        self.disk_usage = items.clone();
                        self.push_status(format!("Disk usage updated: {} item(s)", items.len()));
                    }
                    SystemEvent::MetricsStackScanned(_) => {
                        // IntegrationsView handles this in on_event
                    }
                    SystemEvent::HubbleTrafficUpdated(_) => {
                        // HubblePanel (React bridge) handles this in on_event; no egui view yet.
                    }
                    SystemEvent::FluxOverviewUpdated(_) => {
                        // FluxView handles this in on_event
                    }
                    SystemEvent::HelmMutationPreviewReady(_)
                    | SystemEvent::HelmMutationApplied(_)
                    | SystemEvent::HelmMutationFailed(_) => {
                        // Helm mutate (React bridge) handles this in on_event; no egui view yet.
                    }
                    SystemEvent::UiConfigLoaded { .. } => {
                        // Only the Tauri UI requests this; the egui UI reads config.json directly.
                    }
                    SystemEvent::CrdCatalogUpdated { .. }
                    | SystemEvent::CrdInstancesUpdated { .. } => {
                        // Custom Resources browsing (React bridge) handles this in on_event; no egui view yet.
                    }
                },

                BackendEvent::ResourceList(ev) => match ev {
                    ResourceListEvent::NamespacesUpdated(namespaces) => {
                        // ContextView handles this via on_event()
                        // Also update the app's namespace filter list
                        self.update_available_namespaces(namespaces.clone());
                    }
                    ResourceListEvent::PortForwardsUpdated(port_forwards) => {
                        self.port_forwards = port_forwards.clone();
                    }
                    // Resource list updates now handled by views via on_event()
                    _ => {}
                },

                BackendEvent::ResourceRead(_ev) => {
                    // Resource details updates now handled by views via on_event()
                }

                BackendEvent::Stream(ev) => {
                    // Other stream events handled by views via on_event()
                    if let StreamEvent::PortForwardTunnel {
                        tunnel_id: _,
                        status,
                        local_endpoint: _,
                    } = ev
                    {
                        // Broadcast to all views so PortForwardsView can update status indicators
                        // regardless of which view is currently active
                        for view in self.views.values_mut() {
                            view.on_event(event);
                        }
                        self.push_status(format!("Port forward status: {:?}", status));
                    }
                }

                BackendEvent::CommandFinished { .. } => {} // skipped above
            }
        }
    }

    /// ContextView already processed the `ContextChanged` event that
    /// triggered this (its own `on_event` resets `selected_namespaces`/
    /// `select_all_namespaces`). Every other cached view holds resource data
    /// fetched under the *previous* cluster context, and nothing else clears
    /// it - views only refresh via their own `on_mount` or a `Refresh`
    /// command, and neither fires just because the underlying cluster
    /// changed. Drop every view except the one we just updated, and reset
    /// `previous_section` so `handle_view_switch` treats the active section
    /// as freshly mounted next frame - recreating it from `Default`
    /// (clearing stale data) and re-triggering its `Refresh` command against
    /// the new context.
    fn reset_views_for_context_switch(&mut self) {
        let context_key = section_keys::context().to_string();
        let context_view = self.views.remove(&context_key);
        self.views.clear();
        if let Some(view) = context_view {
            self.views.insert(context_key, view);
        }
        self.previous_section = None;
    }
}
