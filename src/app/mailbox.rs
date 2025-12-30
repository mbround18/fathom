use super::*;
use crate::protocol::{ResourceListEvent, StreamEvent};

impl FathomApp {
    pub(super) fn check_mailbox(&mut self) {
        // First, handle view switching and lifecycle
        self.handle_view_switch();

        while let Ok(envelope) = self.gui_rx.try_recv() {
            let event = &envelope.event;

            // Route event to active view
            if let Some(view) = self.views.get_mut(&self.active_section.to_string()) {
                view.on_event(event);
            }

            match envelope.event {
                BackendEvent::System(ev) => match ev {
                    SystemEvent::BackendStarted => {
                        self.push_status("Backend started.".to_owned());
                    }
                    SystemEvent::Notification { level, message } => match level {
                        NotificationLevel::Info => self.push_status(message),
                        NotificationLevel::Warn => self.push_status(format!("WARN: {}", message)),
                        NotificationLevel::Error => self.push_status(format!("ERROR: {}", message)),
                    },
                    SystemEvent::ContextsUpdated { contexts, current } => {
                        self.contexts = contexts;
                        self.current_context = current;
                    }
                    SystemEvent::ContextChanged { current } => {
                        self.current_context = current;

                        // Views handle their own state cleanup via on_unmount/on_mount lifecycle

                        // Backend resets namespace filter to "all" on context switch.
                        self.select_all_namespaces = true;
                        self.selected_namespaces.clear();
                    }
                    SystemEvent::NamespaceFilterChanged { namespaces } => {
                        if namespaces.is_empty() {
                            self.select_all_namespaces = true;
                            self.selected_namespaces.clear();
                        } else {
                            self.select_all_namespaces = false;
                            self.selected_namespaces = namespaces.into_iter().collect();
                        }
                    }
                    SystemEvent::PrometheusConnectionTest { success, message } => {
                        self.prometheus_testing = false;
                        if success {
                            self.prometheus_connected = true;
                            self.push_status("Prometheus connected successfully".to_owned());
                        } else {
                            self.prometheus_connected = false;
                            self.push_status(format!("Prometheus connection failed: {}", message));
                        }
                    }
                    SystemEvent::GrafanaConnectionTest { success, message } => {
                        self.grafana_testing = false;
                        if success {
                            self.grafana_connected = true;
                            self.push_status("Grafana connected successfully".to_owned());
                        } else {
                            self.grafana_connected = false;
                            self.push_status(format!("Grafana connection failed: {}", message));
                        }
                    }
                },

                BackendEvent::ResourceList(ev) => match ev {
                    ResourceListEvent::NamespacesUpdated(namespaces) => {
                        self.namespaces = namespaces;
                        if self.select_all_namespaces {
                            self.selected_namespaces.clear();
                        }
                    }
                    ResourceListEvent::PortForwardsUpdated(port_forwards) => {
                        self.port_forwards = port_forwards;
                    }
                    ResourceListEvent::PodsUpdated(pods) => {
                        self.pods = pods;
                    }
                    ResourceListEvent::DeploymentsUpdated(items) => {
                        self.deployments = items;
                    }
                    ResourceListEvent::DaemonSetsUpdated(items) => {
                        self.daemonsets = items;
                    }
                    ResourceListEvent::StatefulSetsUpdated(items) => {
                        self.statefulsets = items;
                    }
                    ResourceListEvent::ReplicaSetsUpdated(items) => {
                        self.replicasets = items;
                    }
                    ResourceListEvent::ReplicationControllersUpdated(items) => {
                        self.replicationcontrollers = items;
                    }
                    ResourceListEvent::JobsUpdated(items) => {
                        self.jobs = items;
                    }
                    ResourceListEvent::CronJobsUpdated(items) => {
                        self.cronjobs = items;
                    }
                },

                BackendEvent::ResourceRead(ev) => {
                    use crate::protocol::ResourceReadEvent;
                    match ev {
                        ResourceReadEvent::PodDetailsUpdated(details) => {
                            self.pod_details = Some(details);
                        }
                        ResourceReadEvent::DeploymentDetailsUpdated(details) => {
                            self.deployment_details = Some(details);
                        }
                        ResourceReadEvent::DaemonSetDetailsUpdated(details) => {
                            self.daemonset_details = Some(details);
                        }
                        ResourceReadEvent::StatefulSetDetailsUpdated(details) => {
                            self.statefulset_details = Some(details);
                        }
                        ResourceReadEvent::ReplicaSetDetailsUpdated(details) => {
                            self.replicaset_details = Some(details);
                        }
                        ResourceReadEvent::ReplicationControllerDetailsUpdated(details) => {
                            self.replicationcontroller_details = Some(details);
                        }
                        ResourceReadEvent::JobDetailsUpdated(details) => {
                            self.job_details = Some(details);
                        }
                        ResourceReadEvent::CronJobDetailsUpdated(details) => {
                            self.cronjob_details = Some(details);
                        }
                        ResourceReadEvent::PodMetricsUpdated(metrics) => {
                            self.pod_metrics = Some(metrics);
                        }
                        ResourceReadEvent::PodMetricsHistoryUpdated(history) => {
                            self.pod_metrics_history = Some(history);
                        }
                    }
                }

                BackendEvent::Stream(ev) => match ev {
                    StreamEvent::PodLogsChunk {
                        stream_id: _,
                        lines,
                    } => {
                        self.pod_logs.extend(lines);
                        if self.pod_logs.len() > 2000 {
                            let drain = self.pod_logs.len() - 2000;
                            self.pod_logs.drain(0..drain);
                        }
                    }
                    StreamEvent::StreamClosed {
                        stream_id: _,
                        reason,
                    } => {
                        self.push_status(format!("Log stream closed: {}", reason));
                    }
                    StreamEvent::PortForwardTunnel {
                        tunnel_id: _,
                        status: _,
                        local_endpoint: _,
                    } => {
                        // TODO: Handle port forward status updates
                        self.push_status("Port forward status updated".to_string());
                    }
                },
            }
        }
    }
}
