//! Search/filter behavior for `Table`-based resource-list views, using
//! ConfigMaps as the target view (same one `gui_harness.rs`'s delete test
//! uses, so the setup is well-understood). See `tests/common/mod.rs` for the
//! shared harness and gotchas.
//!
//! The search box (a plain `egui::TextEdit` with hint text "Filter
//! results…", drawn by `Table::render_table`) exposes no distinguishing
//! accessible label or value while empty - it's a bare `Role::TextInput`
//! with `value: ""` - so tests here find it via `get_by_role`.
//!
//! `ConfigMapsView` (`src/app/views/configmaps.rs`) used to render a
//! *second*, redundant text input above this one, also bound to
//! `table.state.search_text` but never wired to `Table::on_filter_change` -
//! writing to the same field is why it never actually diverged from the
//! working box. Found while writing this file (a `get_all_by_role` dump
//! showed two `Role::TextInput` nodes for one search field) and removed from
//! all 25 `Table`-based views that had it, since they were all generated
//! from the same template. See `git log -- src/app/views/configmaps.rs` for
//! the fix.

mod common;

use accesskit::Role;
use common::setup;
use egui_kittest::kittest::Queryable;
use fathom::protocol::{BackendEvent, ConfigMapSummary, ResourceListEvent, Route, section_keys};

fn configmap(name: &str, namespace: &str) -> ConfigMapSummary {
    ConfigMapSummary {
        name: name.to_owned(),
        namespace: namespace.to_owned(),
        keys_count: 1,
    }
}

/// Populates the ConfigMaps table with three rows chosen so a naive/buggy
/// substring match could plausibly go wrong: "alpha-config" and
/// "alpha-secrets-config" share the "alpha-" prefix, "alpha-config" and
/// "beta-config" share the "-config" suffix, and "alpha-secrets-config"
/// lives in a different namespace ("staging") than the other two ("prod").
fn setup_configmaps() -> common::TestApp {
    let mut app = setup();
    app.navigate("Config", "  ConfigMaps");

    app.push_event(
        Route::global(section_keys::config_configmaps()),
        BackendEvent::ResourceList(ResourceListEvent::ConfigMapsUpdated(vec![
            configmap("alpha-config", "prod"),
            configmap("beta-config", "prod"),
            configmap("alpha-secrets-config", "staging"),
        ])),
    );

    app
}

/// Basic filtering: typing "beta" should leave only "beta-config" visible,
/// hiding both "alpha-config" and "alpha-secrets-config".
#[test]
fn typing_a_query_filters_to_matching_rows_only() {
    let mut app = setup_configmaps();

    assert!(app.harness.query_by_label("alpha-config").is_some());
    assert!(app.harness.query_by_label("beta-config").is_some());
    assert!(app.harness.query_by_label("alpha-secrets-config").is_some());

    app.harness.get_by_role(Role::TextInput).click();
    app.harness.run_steps(1);
    app.harness.get_by_role(Role::TextInput).type_text("beta");
    app.harness.run_steps(2);

    assert!(
        app.harness.query_by_label("alpha-config").is_none(),
        "expected 'alpha-config' to be filtered out by query 'beta'"
    );
    assert!(
        app.harness.query_by_label("alpha-secrets-config").is_none(),
        "expected 'alpha-secrets-config' to be filtered out by query 'beta'"
    );
    assert!(
        app.harness.query_by_label("beta-config").is_some(),
        "expected 'beta-config' to remain visible for query 'beta'"
    );
}

/// A query that shares a substring with an unrelated row's suffix
/// ("-config" is common to all three) should still only match rows whose
/// name or namespace actually contains the full query - "alpha" should
/// match both "alpha-config" and "alpha-secrets-config" (name prefix) but
/// not "beta-config".
#[test]
fn query_matches_by_name_prefix_across_namespaces() {
    let mut app = setup_configmaps();

    app.harness.get_by_role(Role::TextInput).click();
    app.harness.run_steps(1);
    app.harness.get_by_role(Role::TextInput).type_text("alpha");
    app.harness.run_steps(2);

    assert!(
        app.harness.query_by_label("alpha-config").is_some(),
        "expected 'alpha-config' to match query 'alpha'"
    );
    assert!(
        app.harness.query_by_label("alpha-secrets-config").is_some(),
        "expected 'alpha-secrets-config' to match query 'alpha'"
    );
    assert!(
        app.harness.query_by_label("beta-config").is_none(),
        "expected 'beta-config' to be filtered out by query 'alpha'"
    );
}

/// A query matching only a namespace (not any name) should filter by that
/// too, since the ConfigMaps search closure checks both fields.
#[test]
fn query_matches_by_namespace() {
    let mut app = setup_configmaps();

    app.harness.get_by_role(Role::TextInput).click();
    app.harness.run_steps(1);
    app.harness
        .get_by_role(Role::TextInput)
        .type_text("staging");
    app.harness.run_steps(2);

    assert!(
        app.harness.query_by_label("alpha-secrets-config").is_some(),
        "expected the 'staging' row to remain visible for query 'staging'"
    );
    assert!(
        app.harness.query_by_label("alpha-config").is_none(),
        "expected a 'prod' row to be filtered out by query 'staging'"
    );
    assert!(
        app.harness.query_by_label("beta-config").is_none(),
        "expected a 'prod' row to be filtered out by query 'staging'"
    );
}

/// Clicking "Clear" after filtering should restore every row - not just
/// visually reset the text box while leaving stale filter state behind.
#[test]
fn clearing_the_search_restores_the_full_list() {
    let mut app = setup_configmaps();

    app.harness.get_by_role(Role::TextInput).click();
    app.harness.run_steps(1);
    app.harness.get_by_role(Role::TextInput).type_text("beta");
    app.harness.run_steps(2);

    assert!(
        app.harness.query_by_label("alpha-config").is_none(),
        "sanity check: query 'beta' should have filtered 'alpha-config' out"
    );

    // Two "Clear" buttons exist on screen: the Table's own (next to the
    // search box) and an unrelated one on the bottom Status Log panel -
    // the Table's is rendered first in tree order.
    app.harness
        .get_all_by_label("Clear")
        .next()
        .expect("Table's Clear button")
        .click();
    app.harness.run_steps(2);

    assert!(
        app.harness.query_by_label("alpha-config").is_some(),
        "expected 'alpha-config' to reappear after clearing the search"
    );
    assert!(
        app.harness.query_by_label("beta-config").is_some(),
        "expected 'beta-config' to still be visible after clearing the search"
    );
    assert!(
        app.harness.query_by_label("alpha-secrets-config").is_some(),
        "expected 'alpha-secrets-config' to reappear after clearing the search"
    );
}

/// Search is documented (via `.to_lowercase()` on both sides in the search
/// closure) to be case-insensitive - an upper-case query should match a
/// lower-case name.
#[test]
fn search_is_case_insensitive() {
    let mut app = setup_configmaps();

    app.harness.get_by_role(Role::TextInput).click();
    app.harness.run_steps(1);
    app.harness.get_by_role(Role::TextInput).type_text("BETA");
    app.harness.run_steps(2);

    assert!(
        app.harness.query_by_label("beta-config").is_some(),
        "expected upper-case query 'BETA' to still match lower-case name 'beta-config'"
    );
    assert!(
        app.harness.query_by_label("alpha-config").is_none(),
        "expected 'alpha-config' to be filtered out by query 'BETA'"
    );
}
