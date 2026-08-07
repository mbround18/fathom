use arboard::Clipboard;
use egui::Ui;
use rfd::FileDialog;

/// Identifies the source of a log stream for pub/sub messaging.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum LogStreamId {
    Pod { namespace: String, name: String },
    Deployment { namespace: String, name: String },
    StatefulSet { namespace: String, name: String },
    DaemonSet { namespace: String, name: String },
    ReplicaSet { namespace: String, name: String },
}

impl LogStreamId {
    pub fn display(&self) -> String {
        match self {
            Self::Pod { namespace, name } => format!("{}/{}", namespace, name),
            Self::Deployment { namespace, name } => format!("deploy/{}/{}", namespace, name),
            Self::StatefulSet { namespace, name } => format!("sts/{}/{}", namespace, name),
            Self::DaemonSet { namespace, name } => format!("ds/{}/{}", namespace, name),
            Self::ReplicaSet { namespace, name } => format!("rs/{}/{}", namespace, name),
        }
    }
}

/// State for a single log stream consumer.
#[derive(Debug, Clone)]
pub struct LogStreamState {
    pub stream_id: LogStreamId,
    /// Backend-assigned numeric id (`protocol::StreamId`) of the stream
    /// currently populating `logs`, once known. Not the same thing as
    /// `stream_id` above (which identifies *what* is being tailed - a pod,
    /// a deployment - not *which* backend streaming task).
    backend_stream_id: Option<u64>,
    pub logs: Vec<String>,
    pub container: Option<String>,
    pub is_running: bool,
    pub autoscroll: bool,
    pub autoscroll_forced_off: bool,
    pub filter: String,
    pub filter_case_sensitive: bool,
    pub filter_include: bool,
}

impl LogStreamState {
    pub fn new(stream_id: LogStreamId) -> Self {
        Self {
            stream_id,
            backend_stream_id: None,
            logs: Vec::new(),
            container: None,
            is_running: false,
            autoscroll: true,
            autoscroll_forced_off: false,
            filter: String::new(),
            filter_case_sensitive: false,
            filter_include: true,
        }
    }

    /// Whether an incoming event tagged with backend stream id `incoming` is
    /// for the stream that should currently be populating `logs`, and if so,
    /// records it as the current one.
    ///
    /// Backend log-tailing tasks are cancelled via `JoinHandle::abort()`,
    /// which cancels the task at its next await point but does not retract
    /// events it already queued for delivery - so switching pods (or
    /// restarting a stream) can leave a handful of `PodLogsChunk` events from
    /// the *old* stream still in flight after a *new* `LogStreamState` has
    /// already been created for the new selection, and they'd otherwise get
    /// silently appended into the wrong pod's log view. Since backend stream
    /// ids are assigned monotonically within a session, rejecting anything
    /// older than the highest id seen so far discards those stragglers
    /// without needing the backend to announce a new stream id in advance.
    pub fn accepts_chunk(&mut self, incoming: u64) -> bool {
        if let Some(current) = self.backend_stream_id
            && incoming < current
        {
            return false;
        }
        self.backend_stream_id = Some(incoming);
        true
    }

    pub fn clear_logs(&mut self) {
        self.logs.clear();
    }

    pub fn add_lines(&mut self, lines: Vec<String>) {
        self.logs.extend(lines);
    }

    pub fn filtered_logs(&self) -> Vec<String> {
        if self.filter.is_empty() {
            return self.logs.clone();
        }

        let needle = if self.filter_case_sensitive {
            self.filter.clone()
        } else {
            self.filter.to_lowercase()
        };

        self.logs
            .iter()
            .filter(|line| {
                let hay = if self.filter_case_sensitive {
                    (*line).clone()
                } else {
                    line.to_lowercase()
                };
                let matched = hay.contains(&needle);
                if self.filter_include {
                    matched
                } else {
                    !matched
                }
            })
            .cloned()
            .collect()
    }

    pub fn filter_count(&self) -> (usize, usize) {
        let total = self.logs.len();
        let filtered = self.filtered_logs().len();
        (filtered, total)
    }
}

/// Generic log viewer component with controls.
pub struct LogViewer;

impl LogViewer {
    /// Render log controls (Start, Stop, Clear, AutoScroll, Save, Copy, Container selector).
    /// Returns tuple of (should_start, should_stop, should_clear, container_changed).
    pub fn render_controls(
        ui: &mut Ui,
        state: &mut LogStreamState,
        available_containers: &[String],
    ) -> (bool, bool, bool, bool) {
        let mut should_start = false;
        let mut should_stop = false;
        let mut should_clear = false;
        let mut container_changed = false;

        ui.horizontal(|ui| {
            if ui.button("Start").clicked() {
                should_start = true;
            }

            if ui.button("Stop").clicked() {
                should_stop = true;
            }

            if ui.button("Clear").clicked() {
                should_clear = true;
                state.clear_logs();
            }

            ui.add_space(12.0);

            // Auto-scroll toggle
            let resp = ui.checkbox(&mut state.autoscroll, "Auto-scroll");
            if resp.changed() {
                state.autoscroll_forced_off = false;
            }

            ui.add_space(12.0);

            // Save logs to file
            if ui.button("Save…").clicked() {
                let content = state.logs.join("\n");
                let default_name =
                    format!("{}-logs.txt", state.stream_id.display().replace('/', "_"));

                if let Some(path) = FileDialog::new()
                    .set_title("Save Logs")
                    .set_file_name(&default_name)
                    .save_file()
                {
                    if let Err(e) = std::fs::write(&path, content) {
                        tracing::warn!("Failed to save logs to {:?}: {}", path, e);
                    } else {
                        tracing::info!("Saved logs to {:?}", path);
                    }
                }
            }

            // Copy logs to clipboard
            if ui.button("Copy").clicked() {
                let content = state.logs.join("\n");
                if let Ok(mut cb) = Clipboard::new() {
                    let _ = cb.set_text(content);
                }
            }

            // Container selector for multi-container resources
            if !available_containers.is_empty() {
                ui.label("Container:");

                let current_container = state.container.as_deref().unwrap_or("(default)");

                egui::ComboBox::from_id_salt(format!("log_container_{:?}", state.stream_id))
                    .selected_text(current_container)
                    .show_ui(ui, |ui| {
                        if ui
                            .selectable_value(&mut state.container, None, "(default)")
                            .changed()
                        {
                            state.clear_logs();
                            container_changed = true;
                        }

                        for container in available_containers {
                            let mut selected = state.container.as_ref() == Some(container);
                            if ui
                                .selectable_value(&mut selected, true, container)
                                .changed()
                            {
                                state.container = Some(container.clone());
                                state.clear_logs();
                                container_changed = true;
                            }
                        }
                    });
            }
        });

        (should_start, should_stop, should_clear, container_changed)
    }

    /// Render log filter controls (search box, case-sensitive toggle, include/exclude mode, counter).
    pub fn render_filter_controls(ui: &mut Ui, state: &mut LogStreamState) {
        ui.horizontal(|ui| {
            ui.label("Filter:");
            ui.add(egui::TextEdit::singleline(&mut state.filter).hint_text("substring…"));
            if ui.button("Clear Filter").clicked() {
                state.filter.clear();
            }
            ui.checkbox(&mut state.filter_case_sensitive, "Match case");

            ui.add_space(8.0);
            let mode_label = if state.filter_include {
                "With"
            } else {
                "Without"
            };
            if ui.button(mode_label).clicked() {
                state.filter_include = !state.filter_include;
            }

            ui.add_space(12.0);
            let (filtered, total) = state.filter_count();
            ui.label(format!("Showing {} of {}", filtered, total));
        });
    }

    /// Render the scrollable logs area with filtering applied.
    pub fn render_logs(
        ui: &mut Ui,
        state: &mut LogStreamState,
    ) -> egui::containers::scroll_area::ScrollAreaOutput<()> {
        let inner = egui::ScrollArea::vertical()
            .id_salt("log_viewer_scroll")
            .auto_shrink([false; 2])
            .stick_to_bottom(state.autoscroll)
            .show(ui, |ui| {
                let filtered_logs = state.filtered_logs();
                for line in filtered_logs {
                    ui.monospace(&line);
                }

                if state.autoscroll {
                    ui.scroll_to_cursor(Some(egui::Align::BOTTOM));
                }
            });

        // Detect scroll position and manage auto-scroll state
        let viewport_height = inner.inner_rect.height();
        let max_offset = (inner.content_size.y - viewport_height).max(0.0);
        let at_bottom = inner.state.offset.y >= max_offset - 1.0;

        if !at_bottom && state.autoscroll {
            state.autoscroll = false;
            state.autoscroll_forced_off = true;
        }

        if at_bottom && state.autoscroll_forced_off && !state.autoscroll {
            state.autoscroll = true;
            state.autoscroll_forced_off = false;
        }

        inner
    }

    /// Convenience method to render full log viewer UI in one call.
    pub fn render_full(
        ui: &mut Ui,
        state: &mut LogStreamState,
        available_containers: &[String],
    ) -> (bool, bool, bool, bool) {
        let (should_start, should_stop, should_clear, container_changed) =
            Self::render_controls(ui, state, available_containers);

        ui.add_space(8.0);
        Self::render_filter_controls(ui, state);
        ui.add_space(8.0);

        Self::render_logs(ui, state);

        (should_start, should_stop, should_clear, container_changed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pod_stream() -> LogStreamState {
        LogStreamState::new(LogStreamId::Pod {
            namespace: "default".to_owned(),
            name: "web-0".to_owned(),
        })
    }

    /// Regression test for a bug found by inspection: `PodsView::on_event`
    /// used to append every `PodLogsChunk` to whatever `pod_logs_stream` was
    /// current, ignoring the event's own `stream_id` entirely. Backend
    /// log-tailing tasks are stopped via `JoinHandle::abort()`, which cancels
    /// the task at its next await point but doesn't retract chunks it had
    /// already queued for delivery - so switching pods mid-stream could
    /// leave a handful of the *previous* pod's log lines still in flight,
    /// arriving after a fresh `LogStreamState` was created for the new
    /// selection, and get silently appended into the wrong pod's log view.
    #[test]
    fn accepts_chunk_rejects_stragglers_from_a_superseded_stream() {
        let mut stream = pod_stream();

        // First chunk for this viewing session establishes the current
        // stream id (5, e.g. from switching to this pod and clicking Start).
        assert!(stream.accepts_chunk(5));
        assert!(
            stream.accepts_chunk(5),
            "more chunks from the same stream keep being accepted"
        );

        // A late-arriving chunk from an older, already-superseded stream
        // (e.g. the previous pod's, aborted but not fully drained) must be
        // rejected, not appended.
        assert!(
            !stream.accepts_chunk(3),
            "a chunk from an older stream id must be rejected"
        );

        // A genuinely newer stream (e.g. the user clicked Start again)
        // should still be accepted and become the new current stream.
        assert!(stream.accepts_chunk(7));
        assert!(
            !stream.accepts_chunk(5),
            "once stream 7 is current, straggler chunks from stream 5 must be rejected too"
        );
    }

    /// The very first chunk a freshly-created `LogStreamState` ever sees
    /// establishes its stream id - there's nothing to compare against yet.
    #[test]
    fn accepts_chunk_always_accepts_the_first_one() {
        let mut stream = pod_stream();
        assert!(stream.accepts_chunk(42));
    }
}
