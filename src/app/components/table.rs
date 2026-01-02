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

/// Table state for sorting and searching
#[derive(Clone, Debug)]
pub struct TableState {
    pub sort_column: Option<usize>,
    pub sort_dir: SortDir,
    pub search_text: String,
}

impl Default for TableState {
    fn default() -> Self {
        Self {
            sort_column: None,
            sort_dir: SortDir::Asc,
            search_text: String::new(),
        }
    }
}

/// Generic sortable and searchable table component
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

    /// Get the displayed items (sorted and filtered)
    pub fn filtered_items(&self, search_fn: fn(&T, &str) -> bool) -> Vec<(usize, T)> {
        let mut items: Vec<(usize, T)> = self
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

    /// Render the table using egui Grid (proper column layout)
    pub fn render_table(
        &mut self,
        ui: &mut egui::Ui,
        columns: &[Column<T>],
        search_fn: fn(&T, &str) -> bool,
        on_row_clicked: Option<fn(usize) -> ()>,
    ) where
        T: Clone,
    {
        // Search bar
        ui.horizontal(|ui| {
            ui.label("Search:");
            ui.text_edit_singleline(&mut self.state.search_text);
        });
        ui.add_space(6.0);

        // Get filtered items
        let mut items = self.filtered_items(search_fn);

        // Table grid layout
        egui::ScrollArea::vertical().show(ui, |ui| {
            egui::Grid::new("table_grid").striped(true).show(ui, |ui| {
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
                if let Some(col_idx) = clicked_column {
                    let sort_value_fn = columns[col_idx].sort_value;
                    items = self.sort_items(col_idx, items, sort_value_fn);
                }

                // Table rows
                for (display_idx, (original_idx, item)) in items.iter().enumerate() {
                    let is_selected = self.selected == Some(display_idx);

                    for column in columns.iter() {
                        let cell_text = (column.render)(item);
                        if ui.selectable_label(is_selected, &cell_text).clicked() {
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
