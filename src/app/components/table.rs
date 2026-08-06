use std::cmp::Ordering;

/// Sort direction for columns
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SortDir {
    Asc,
    Desc,
}

impl SortDir {
    pub fn toggle(self) -> Self {
        match self {
            SortDir::Asc => SortDir::Desc,
            SortDir::Desc => SortDir::Asc,
        }
    }
}

/// Column definition for a table
pub struct Column<T> {
    pub header: String,
    pub width: f32,
    /// Function to extract the sortable value (for sorting)
    pub sort_value: fn(&T) -> String,
    /// Function to render the cell content
    pub render: fn(&T) -> String,
}

/// Table state for sorting, searching, and multi-selection
#[derive(Clone, Debug)]
pub struct TableState {
    pub sort_column: Option<usize>,
    pub sort_dir: SortDir,
    pub search_text: String,
    /// Indices of selected rows (in filtered view)
    pub selected_rows: Vec<usize>,
    /// Whether all currently visible (filtered) rows are selected
    pub all_filtered_selected: bool,
}

impl Default for TableState {
    fn default() -> Self {
        Self {
            sort_column: None,
            sort_dir: SortDir::Asc,
            search_text: String::new(),
            selected_rows: Vec::new(),
            all_filtered_selected: false,
        }
    }
}

/// Generic sortable and searchable table component with multi-select support
pub struct Table<T: Clone> {
    pub items: Vec<T>,
    pub state: TableState,
    pub selected: Option<usize>,
}

impl<T: Clone> Table<T> {
    pub fn new(items: Vec<T>) -> Self {
        Self {
            items,
            state: TableState::default(),
            selected: None,
        }
    }

    /// Update items (useful when data changes)
    pub fn set_items(&mut self, items: Vec<T>) {
        self.items = items;
    }

    /// Toggle selection of a specific row
    pub fn toggle_selection(&mut self, row_idx: usize) {
        if let Some(pos) = self.state.selected_rows.iter().position(|&x| x == row_idx) {
            self.state.selected_rows.remove(pos);
        } else {
            self.state.selected_rows.push(row_idx);
        }
        self.state.all_filtered_selected = false;
    }

    /// Select all filtered rows
    pub fn select_all_filtered(&mut self, filtered_count: usize) {
        self.state.selected_rows.clear();
        for i in 0..filtered_count {
            self.state.selected_rows.push(i);
        }
        self.state.all_filtered_selected = true;
    }

    /// Deselect all rows
    pub fn deselect_all(&mut self) {
        self.state.selected_rows.clear();
        self.state.all_filtered_selected = false;
    }

    /// Check if a row is selected
    pub fn is_row_selected(&self, row_idx: usize) -> bool {
        self.state.selected_rows.contains(&row_idx)
    }

    /// Get count of selected rows
    pub fn selected_count(&self) -> usize {
        self.state.selected_rows.len()
    }

    /// Check if all visible rows are selected
    pub fn is_all_selected(&self) -> bool {
        self.state.all_filtered_selected
    }

    /// Get selected row indices
    pub fn get_selected_indices(&self) -> Vec<usize> {
        self.state.selected_rows.clone()
    }

    /// Return all currently selected items in their filtered display order.
    /// Uses the same search_fn as render_table to ensure index alignment.
    pub fn get_selected_items(&self, search_fn: fn(&T, &str) -> bool) -> Vec<T> {
        let filtered: Vec<(usize, T)> = self
            .items
            .iter()
            .enumerate()
            .filter(|(_, item)| {
                self.state.search_text.is_empty() || search_fn(item, &self.state.search_text.to_lowercase())
            })
            .map(|(i, item)| (i, item.clone()))
            .collect();

        self.state
            .selected_rows
            .iter()
            .filter_map(|&display_idx| filtered.get(display_idx).map(|(_, item)| item.clone()))
            .collect()
    }

    /// Clear selections when filters change
    pub fn on_filter_change(&mut self) {
        self.deselect_all();
    }

    /// Get the displayed items (sorted and filtered)
    pub fn filtered_items(&self, search_fn: fn(&T, &str) -> bool) -> Vec<(usize, T)> {
        let items: Vec<(usize, T)> = self
            .items
            .iter()
            .enumerate()
            .filter(|(_, item)| {
                if self.state.search_text.is_empty() {
                    true
                } else {
                    search_fn(item, &self.state.search_text.to_lowercase())
                }
            })
            .map(|(idx, item)| (idx, item.clone()))
            .collect();

        items
    }

    /// Sort items by column
    pub fn sort_items<F>(
        &mut self,
        column_idx: usize,
        mut items: Vec<(usize, T)>,
        sort_value_fn: F,
    ) -> Vec<(usize, T)>
    where
        F: Fn(&T) -> String,
    {
        // Toggle sort if clicking same column
        if self.state.sort_column == Some(column_idx) {
            self.state.sort_dir = self.state.sort_dir.toggle();
        } else {
            self.state.sort_column = Some(column_idx);
            self.state.sort_dir = SortDir::Asc;
        }

        items.sort_by(|a, b| {
            let a_val = sort_value_fn(&a.1);
            let b_val = sort_value_fn(&b.1);

            let cmp = compare_mixed(&a_val, &b_val);
            match self.state.sort_dir {
                SortDir::Asc => cmp,
                SortDir::Desc => cmp.reverse(),
            }
        });

        items
    }

    /// Render the table using egui Grid with multi-select checkboxes
    pub fn render_table(
        &mut self,
        ui: &mut egui::Ui,
        columns: &[Column<T>],
        search_fn: fn(&T, &str) -> bool,
        on_row_clicked: Option<fn(usize) -> ()>,
    ) where
        T: Clone,
    {
        // Search bar with placeholder and reset button
        ui.horizontal(|ui| {
            ui.label("Search:");
            let resp = ui.add(
                egui::TextEdit::singleline(&mut self.state.search_text)
                    .hint_text("Filter results…"),
            );
            if resp.changed() {
                self.on_filter_change();
            }

            if ui.button("Clear").clicked() {
                self.state.search_text.clear();
                self.on_filter_change();
            }

            ui.add_space(12.0);
            ui.separator();
            ui.label("Sort:");

            // Sort column combobox
            let current_col_name = if let Some(col_idx) = self.state.sort_column {
                columns
                    .get(col_idx)
                    .map(|c| c.header.clone())
                    .unwrap_or_else(|| "—".to_string())
            } else {
                "—".to_string()
            };

            let mut selected_col_idx = self.state.sort_column;
            egui::ComboBox::from_id_salt("table_sort_column")
                .selected_text(&current_col_name)
                .show_ui(ui, |ui| {
                    for (idx, col) in columns.iter().enumerate() {
                        ui.selectable_value(&mut selected_col_idx, Some(idx), &col.header);
                    }
                });

            if selected_col_idx != self.state.sort_column {
                self.state.sort_column = selected_col_idx;
                self.state.sort_dir = SortDir::Asc;
            }

            // Direction toggle button
            let dir_text = match self.state.sort_dir {
                SortDir::Asc => "▲",
                SortDir::Desc => "▼",
            };
            if ui.button(dir_text).clicked() {
                self.state.sort_dir = self.state.sort_dir.toggle();
            }

            if ui.button("Reset Sort").clicked() {
                self.state.sort_column = None;
                self.state.sort_dir = SortDir::Asc;
            }
        });
        ui.add_space(6.0);

        // Get filtered items
        let items = self.filtered_items(search_fn);
        let filtered_count = items.len();

        // Table grid layout
        egui::ScrollArea::vertical().show(ui, |ui| {
            egui::Grid::new("table_grid").striped(true).show(ui, |ui| {
                // Checkbox header (3-state: unchecked, partial, all-checked)
                let header_checkbox_state = if self.state.selected_rows.is_empty() {
                    "☐"
                } else if self.state.all_filtered_selected {
                    "☑"
                } else {
                    "☒" // Partial selection
                };

                if ui.button(header_checkbox_state).clicked() {
                    if self.state.all_filtered_selected {
                        self.deselect_all();
                    } else {
                        self.select_all_filtered(filtered_count);
                    }
                }

                // Column headers with sort indicators
                let mut clicked_column: Option<usize> = None;
                for (col_idx, column) in columns.iter().enumerate() {
                    let mut header_text = column.header.clone();

                    // Add sort indicator
                    if self.state.sort_column == Some(col_idx) {
                        header_text.push_str(match self.state.sort_dir {
                            SortDir::Asc => " ▲",
                            SortDir::Desc => " ▼",
                        });
                    }

                    if ui.button(&header_text).clicked() {
                        clicked_column = Some(col_idx);
                    }
                }
                ui.end_row();

                // Handle sort after headers
                let sorted_items = if let Some(col_idx) = clicked_column {
                    let sort_value_fn = columns[col_idx].sort_value;
                    let sorted = items;
                    self.sort_items(col_idx, sorted, sort_value_fn)
                } else {
                    items
                };

                // Table rows
                for (display_idx, (original_idx, item)) in sorted_items.iter().enumerate() {
                    let is_selected = self.is_row_selected(display_idx);

                    // Checkbox for row selection
                    let checkbox = if is_selected { "☑" } else { "☐" };
                    if ui.button(checkbox).clicked() {
                        self.toggle_selection(display_idx);
                    }

                    let is_highlighted = self.selected == Some(display_idx);

                    for column in columns.iter() {
                        let cell_text = (column.render)(item);
                        if ui.selectable_label(is_highlighted, &cell_text).clicked() {
                            self.selected = Some(display_idx);
                            if let Some(callback) = on_row_clicked {
                                callback(*original_idx);
                            }
                        }
                    }
                    ui.end_row();
                }
            });
        });
    }
}

/// Mix-typed comparison (handles both numeric and string comparison)
fn compare_mixed(a: &str, b: &str) -> Ordering {
    // Try numeric comparison first
    if let (Ok(a_num), Ok(b_num)) = (a.parse::<i32>(), b.parse::<i32>()) {
        return a_num.cmp(&b_num);
    }

    // Fall back to string comparison
    a.cmp(b)
}
