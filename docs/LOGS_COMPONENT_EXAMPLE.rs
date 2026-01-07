// Example: Adding logs to a Deployments view using the generic LogViewer component
// This demonstrates the reusability pattern for extending logs to other resource types.

use crate::app::components::{LogStreamId, LogStreamState, LogViewer};
use crate::protocol::{BackendCmd, BackendEvent, StreamEvent};

// In your deployment view struct:
pub struct DeploymentsView {
    // ... other fields ...
    pub deployment_logs_stream: Option<LogStreamState>,
}

impl DeploymentsView {
    pub fn on_deployment_selected(&mut self, namespace: &str, name: &str) {
        // Initialize logs stream for the selected deployment
        let stream_id = LogStreamId::Deployment {
            namespace: namespace.to_string(),
            name: name.to_string(),
        };
        self.deployment_logs_stream = Some(LogStreamState::new(stream_id));
    }

    pub fn render_logs_tab(&mut self, ui: &mut egui::Ui) {
        if let Some(stream) = &mut self.deployment_logs_stream {
            // Get available containers from deployment pods or details
            let containers = vec![]; // Fetch from your deployment details

            // Render full log viewer in one call
            let (should_start, should_stop, _should_clear, _container_changed) =
                LogViewer::render_full(ui, stream, &containers);

            // Handle start action - sends backend command to stream logs from deployment
            if should_start {
                // This would route to deployment-specific log streaming in backend
                // Logs from all replicas' pods could be aggregated here
                tracing::info!("Start streaming logs for deployment");
            }

            if should_stop {
                tracing::info!("Stop streaming logs for deployment");
            }
        }
    }

    pub fn handle_event(&mut self, event: BackendEvent) {
        // Handle logs events - same pattern as pods
        if let BackendEvent::Stream(StreamEvent::PodLogsChunk { lines, .. }) = event {
            if let Some(stream) = &mut self.deployment_logs_stream {
                stream.add_lines(lines);
                // Respect max logs limit
                const MAX_LOGS: usize = 1000;
                if stream.logs.len() > MAX_LOGS {
                    stream.logs.drain(0..stream.logs.len() - MAX_LOGS);
                }
            }
        }
    }

    pub fn on_deployment_deselected(&mut self) {
        self.deployment_logs_stream = None;
    }
}

// Alternative: If you want to manage multiple concurrent log streams for different resources,
// you can use a HashMap:

use std::collections::HashMap;

pub struct MultiResourceView {
    pub log_streams: HashMap<LogStreamId, LogStreamState>,
}

impl MultiResourceView {
    pub fn new() -> Self {
        Self {
            log_streams: HashMap::new(),
        }
    }

    pub fn start_logs(&mut self, stream_id: LogStreamId) {
        self.log_streams.insert(stream_id.clone(), LogStreamState::new(stream_id));
    }

    pub fn get_logs(&mut self, stream_id: &LogStreamId) -> Option<&mut LogStreamState> {
        self.log_streams.get_mut(stream_id)
    }

    pub fn render_logs_for_stream(
        &mut self,
        ui: &mut egui::Ui,
        stream_id: LogStreamId,
        containers: &[String],
    ) {
        if let Some(stream) = self.log_streams.get_mut(&stream_id) {
            let (should_start, should_stop, _clear, _container_changed) =
                LogViewer::render_full(ui, stream, containers);

            if should_start {
                tracing::info!("Start streaming logs: {}", stream_id.display());
            }
            if should_stop {
                tracing::info!("Stop streaming logs: {}", stream_id.display());
            }
        }
    }

    pub fn handle_event(&mut self, event: BackendEvent) {
        if let BackendEvent::Stream(StreamEvent::PodLogsChunk { lines, .. }) = event {
            // Route to all active streams (or filter by stream_id if protocol is enhanced)
            for stream in self.log_streams.values_mut() {
                stream.add_lines(lines.clone());
                const MAX_LOGS: usize = 1000;
                if stream.logs.len() > MAX_LOGS {
                    stream.logs.drain(0..stream.logs.len() - MAX_LOGS);
                }
            }
        }
    }

    pub fn stop_logs(&mut self, stream_id: &LogStreamId) {
        self.log_streams.remove(stream_id);
    }
}
