use super::*;

impl FathomApp {
    pub(super) fn check_mailbox(&mut self) {
        while let Ok(envelope) = self.gui_rx.try_recv() {
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

                        // Global invalidation: switching clusters invalidates selections and caches.
                        self.selected_pod = None;
                        self.pod_details = None;
                        self.pod_metrics = None;
                        self.pod_logs.clear();

                        self.selected_deployment = None;
                        self.deployment_details = None;

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
                },

                BackendEvent::ResourceList(ev) => match ev {
                    ResourceListEvent::NamespacesUpdated(namespaces) => {
                        self.namespaces = namespaces;
                        if self.select_all_namespaces {
                            self.selected_namespaces.clear();
                        }
                    }
                    ResourceListEvent::PodsUpdated(pods) => {
                        self.pods = pods;
                    }
                    ResourceListEvent::DeploymentsUpdated(deployments) => {
                        self.deployments = deployments;
                    }
                },

                BackendEvent::ResourceRead(ev) => match ev {
                    ResourceReadEvent::PodDetailsUpdated(details) => {
                        self.pod_details = Some(details);
                    }
                    ResourceReadEvent::PodMetricsUpdated(metrics) => {
                        self.pod_metrics = Some(metrics);
                    }
                    ResourceReadEvent::DeploymentDetailsUpdated(details) => {
                        self.deployment_details = Some(details);
                    }
                },

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
                },
            }
        }
    }
}
