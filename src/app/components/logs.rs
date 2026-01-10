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
