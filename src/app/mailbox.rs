use super::*;
use crate::protocol::{ResourceListEvent, StreamEvent};

impl FathomApp {
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
                        // ContextView handles this in on_event
                        // Views handle their own state cleanup via on_unmount/on_mount lifecycle
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

                BackendEvent::Stream(ev) => match ev {
                    StreamEvent::PortForwardTunnel {
                        tunnel_id: _,
                        status: _,
                        local_endpoint: _,
                    } => {
                        // TODO: Handle port forward status updates
                        self.push_status("Port forward status updated".to_string());
                    }
                    // Other stream events handled by views via on_event()
                    _ => {}
                },
            }
        }
    }
}
