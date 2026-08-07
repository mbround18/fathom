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
                self.state.search_text.is_empty()
                    || search_fn(item, &self.state.search_text.to_lowercase())
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

    /// Get the displayed items: filtered by search text, in whatever order
    /// `self.items` currently holds. Sorting reorders `self.items` itself
    /// (see `apply_sort`) rather than being applied transiently here, so
    /// this - and everything built on it, including `get_selected_items` -
    /// always agrees with what `render_table` actually shows the user.
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

    /// Reorder `self.items` in place to match the persisted
    /// `sort_column`/`sort_dir`. No-op if no sort column is set. Sorting the
    /// canonical backing store (rather than a transient display-only copy)
    /// is what keeps `filtered_items`, `get_selected_items`, and
    /// `render_table`'s row order in agreement - previously, sort order was
    /// only ever computed for the exact frame a header was clicked and
    /// discarded immediately after, and `get_selected_items` never applied
    /// it at all, so bulk actions after any sort resolved to the wrong items.
    fn apply_sort(&mut self, columns: &[Column<T>]) {
        let Some(col_idx) = self.state.sort_column else {
            return;
        };
        let Some(column) = columns.get(col_idx) else {
            return;
        };
        let sort_value_fn = column.sort_value;
        let dir = self.state.sort_dir;
        self.items.sort_by(|a, b| {
            let cmp = compare_mixed(&sort_value_fn(a), &sort_value_fn(b));
            match dir {
                SortDir::Asc => cmp,
                SortDir::Desc => cmp.reverse(),
            }
        });
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

        // Apply whatever sort was already in effect from the previous frame
        // (or set via the Sort combobox above) before computing what to
        // show - this is what makes sort "sticky" instead of a one-frame
        // flash on the exact click.
        self.apply_sort(columns);
        let filtered_count = self.filtered_items(search_fn).len();

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

                // Toggle sort if clicking the already-sorted column, else
                // switch to sorting by the newly clicked one ascending.
                if let Some(col_idx) = clicked_column {
                    if self.state.sort_column == Some(col_idx) {
                        self.state.sort_dir = self.state.sort_dir.toggle();
                    } else {
                        self.state.sort_column = Some(col_idx);
                        self.state.sort_dir = SortDir::Asc;
                    }
                    self.apply_sort(columns);
                }

                let items = self.filtered_items(search_fn);

                // Table rows
                for (display_idx, (original_idx, item)) in items.iter().enumerate() {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Debug, PartialEq)]
    struct Item {
        name: String,
    }

    fn item(name: &str) -> Item {
        Item {
            name: name.to_owned(),
        }
    }

    fn no_filter(_item: &Item, _search: &str) -> bool {
        true
    }

    fn columns() -> Vec<Column<Item>> {
        vec![Column {
            header: "Name".to_owned(),
            width: 100.0,
            sort_value: |i: &Item| i.name.clone(),
            render: |i: &Item| i.name.clone(),
        }]
    }

    /// Regression test for a bug found via this component's GUI tests:
    /// `render_table` used to only compute a sorted display order on the
    /// exact frame a column header was clicked (a transient local variable,
    /// gated on `clicked_column` being `Some` that one frame) - every other
    /// frame it fell back to natural order, since nothing ever consulted the
    /// *persisted* `sort_column`/`sort_dir`. Since `FathomApp::update` calls
    /// `ctx.request_repaint()` unconditionally every frame, that one sorted
    /// frame was imperceptible: clicking a column header (or picking one
    /// from the Sort combobox) appeared to do nothing.
    ///
    /// Fixed by `apply_sort`, which reorders `self.items` itself based on
    /// persisted state, called at the top of every `render_table` frame -
    /// so `filtered_items` (and everything built on it) now agrees with
    /// whatever was last sorted.
    #[test]
    fn filtered_items_respects_persisted_sort_state() {
        let mut table = Table::new(vec![item("charlie"), item("alpha"), item("bravo")]);
        table.state.sort_column = Some(0);
        table.state.sort_dir = SortDir::Asc;
        table.apply_sort(&columns());

        let names: Vec<String> = table
            .filtered_items(no_filter)
            .into_iter()
            .map(|(_, i)| i.name)
            .collect();

        assert_eq!(
            names,
            vec!["alpha", "bravo", "charlie"],
            "filtered_items should reflect the persisted sort_column/sort_dir"
        );
    }

    /// Regression test for a bug found via this component's GUI tests:
    /// `render_table` stored row checkboxes' indices (`toggle_selection`)
    /// against the *sorted* display order the user actually saw and clicked
    /// against, but `get_selected_items` independently rebuilt its own
    /// filtered list straight from `self.items` in natural (unsorted) order
    /// and read `selected_rows` against *that* - so after any sort, bulk
    /// actions ("Delete N selected") resolved to whatever items happened to
    /// sit at those positions in unsorted order, not the ones the user
    /// actually checked. Fixed by sorting `self.items` itself (`apply_sort`)
    /// so every consumer shares one canonical order.
    #[test]
    fn get_selected_items_matches_the_sorted_order_the_user_saw() {
        let mut table = Table::new(vec![item("charlie"), item("alpha"), item("bravo")]);

        table.state.sort_column = Some(0);
        table.state.sort_dir = SortDir::Asc;
        table.apply_sort(&columns());

        let sorted_names: Vec<String> = table
            .filtered_items(no_filter)
            .into_iter()
            .map(|(_, i)| i.name)
            .collect();
        assert_eq!(
            sorted_names,
            vec!["alpha", "bravo", "charlie"],
            "sanity check: apply_sort should sort ascending by name"
        );

        // User sees the sorted list and checks the first row, "alpha".
        table.toggle_selection(0);

        let selected: Vec<String> = table
            .get_selected_items(no_filter)
            .into_iter()
            .map(|i| i.name)
            .collect();

        assert_eq!(
            selected,
            vec!["alpha".to_owned()],
            "expected the item the user actually saw/checked at display position 0 \
             (\"alpha\", after sorting)"
        );
    }

    /// Toggling sort direction twice on the same column (ascending, then
    /// descending) should reverse the order, not just toggle back and forth
    /// between "sorted" and "unsorted".
    #[test]
    fn sort_direction_toggle_reverses_order() {
        let mut table = Table::new(vec![item("bravo"), item("alpha"), item("charlie")]);
        let cols = columns();

        table.state.sort_column = Some(0);
        table.state.sort_dir = SortDir::Asc;
        table.apply_sort(&cols);
        let asc: Vec<String> = table
            .filtered_items(no_filter)
            .into_iter()
            .map(|(_, i)| i.name)
            .collect();
        assert_eq!(asc, vec!["alpha", "bravo", "charlie"]);

        table.state.sort_dir = SortDir::Desc;
        table.apply_sort(&cols);
        let desc: Vec<String> = table
            .filtered_items(no_filter)
            .into_iter()
            .map(|(_, i)| i.name)
            .collect();
        assert_eq!(desc, vec!["charlie", "bravo", "alpha"]);
    }

    /// Filtering should still narrow the (now-sorted) canonical order rather
    /// than reintroducing natural order.
    #[test]
    fn filtered_items_stays_sorted_after_filtering() {
        let mut table = Table::new(vec![item("charlie"), item("alpha"), item("bravo")]);
        table.state.sort_column = Some(0);
        table.state.sort_dir = SortDir::Asc;
        table.apply_sort(&columns());
        table.state.search_text = "a".to_owned();

        let search_fn = |i: &Item, s: &str| i.name.to_lowercase().contains(s);
        let names: Vec<String> = table
            .filtered_items(search_fn)
            .into_iter()
            .map(|(_, i)| i.name)
            .collect();

        assert_eq!(names, vec!["alpha", "bravo", "charlie"]);
    }
}
