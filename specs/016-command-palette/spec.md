# Feature Specification: Command Palette

**Feature Branch**: `016-command-palette`

**Created**: 2026-09-23

**Status**: Draft

**Input**: User description: "Command palette — Cmd/Ctrl+K opens a fuzzy-searchable palette to jump to any resource, namespace, or tool pane (kind:name syntax, e.g. \"pod:my-app\"), reusing the existing openInspector/nav store actions. Pure frontend feature — no backend/protocol changes needed. Low risk, cheapest high-perceived-power win identified in competitor research (k9s has this)."

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Jump to a known resource by name (Priority: P1)

An operator working across a cluster with dozens of namespaces and hundreds of resources knows the name (or part of the name) of the pod, deployment, or other object they want to inspect, but doesn't want to navigate through namespace filters and list panes to find it. They press Cmd/Ctrl+K, type a few characters of the resource's name (optionally prefixed with its kind, e.g. "pod:my-app"), see a ranked list of matches drawn from resources currently loaded in the app, and select one to jump straight to its Inspector.

**Why this priority**: This is the entire value proposition of the feature — direct, keyboard-driven navigation to a specific resource is the "k9s-style" win being targeted. Without it there is no feature.

**Independent Test**: With the app connected to a cluster and resources loaded, pressing Cmd/Ctrl+K, typing a substring of a known resource's name, and selecting the top match opens the Inspector for that exact resource — fully testable without namespace-jump or pane-jump behavior existing yet.

**Acceptance Scenarios**:

1. **Given** the app is connected and has resources loaded, **When** the operator presses Cmd/Ctrl+K, **Then** the command palette opens with an empty, focused search input.
2. **Given** the palette is open, **When** the operator types a substring that matches part of a known resource's name (e.g. "my-app"), **Then** matching resources appear in the results list, ranked with closer/stronger matches first.
3. **Given** the operator types using `kind:name` syntax (e.g. "pod:my-app"), **When** results are shown, **Then** only resources of the specified kind matching "my-app" appear.
4. **Given** results are shown, **When** the operator selects a result (via click or Enter on the highlighted item), **Then** the palette closes and the Inspector opens showing that exact resource.
5. **Given** the palette is open, **When** the operator presses Escape, **Then** the palette closes without navigating anywhere.

---

### User Story 2 - Jump to a namespace (Priority: P2)

An operator wants to quickly switch the active namespace filter without opening the namespace dropdown and scrolling/searching through a long list. They open the palette, type part of a namespace name, and selecting it applies that namespace as the active filter the same way the existing namespace selector would.

**Why this priority**: High-value but secondary to resource jump — namespace switching already has an existing (if slower) UI path, whereas jumping directly to a resource by name has no fast path today.

**Independent Test**: With multiple namespaces available, opening the palette, typing a namespace substring, and selecting the matching namespace result updates the active namespace filter to that namespace — independently testable without resource or pane results existing.

**Acceptance Scenarios**:

1. **Given** the palette is open, **When** the operator types text matching a namespace name, **Then** a namespace result (visually distinguished from resource/pane results) appears in the list.
2. **Given** a namespace result is selected, **When** the operator confirms it, **Then** the active namespace filter changes to that namespace and the palette closes.

---

### User Story 3 - Jump to a tool pane (Priority: P3)

An operator wants to jump directly to a named tool/section of the app (e.g. "Settings", "Port Forwards", "Available Agents") without using the sidebar navigation, by typing its name into the same palette they already use for resources and namespaces.

**Why this priority**: Nice-to-have completeness; lowest priority because pane navigation already has a persistent, low-friction sidebar affordance, so the palette's marginal benefit here is smaller than for resource/namespace jump.

**Independent Test**: Opening the palette, typing a pane's display name, and selecting the matching pane result navigates the app to that pane — independently testable without resource or namespace results existing.

**Acceptance Scenarios**:

1. **Given** the palette is open, **When** the operator types text matching a known pane's name (e.g. "settings"), **Then** a pane result (visually distinguished from other result types) appears in the list.
2. **Given** a pane result is selected, **When** the operator confirms it, **Then** the app navigates to that pane and the palette closes.

---

### Edge Cases

- What happens when the search query matches nothing? The palette shows a clear "no results" state rather than an empty list that looks broken.
- What happens when the operator opens the palette with no cluster connected / no resources loaded yet? Namespace and pane results (which don't depend on live resource data) still work; resource results show an appropriate empty/unavailable state instead of erroring.
- What happens when the operator types a `kind:` prefix that doesn't match any known kind (e.g. "xyz:foo")? No resource results are shown for that filter rather than falling back to an unfiltered search.
- What happens when Cmd/Ctrl+K is pressed while focus is inside a text input (e.g. a search box or a form field)? The palette still opens and takes focus, consistent with standard command-palette conventions, without corrupting the underlying input's content.
- What happens when the operator presses Cmd/Ctrl+K while the palette is already open? It stays open (idempotent), or toggles closed — either is acceptable as long as behavior is consistent and documented.
- What happens on very large result sets (e.g. thousands of pods across all namespaces)? Results are capped to a reasonable number of top matches so the list stays fast and scannable, rather than rendering everything.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: System MUST open a command palette overlay when the operator presses Cmd+K (macOS) or Ctrl+K (Windows/Linux), from anywhere in the app.
- **FR-002**: System MUST close the palette when the operator presses Escape, selects a result, or clicks outside the palette, without changing the operator's current view unless a result was selected.
- **FR-003**: System MUST provide a single text input in the palette that performs fuzzy, substring-tolerant matching against result names as the operator types, updating results live (no explicit submit step required).
- **FR-004**: System MUST support a `kind:name` query syntax (e.g. "pod:my-app") that restricts resource results to the specified kind before applying the name match.
- **FR-005**: System MUST search across three result categories: cluster resources (currently loaded/known to the app), namespaces (available in the current context), and tool/settings panes (the app's existing navigable sections).
- **FR-006**: System MUST visually distinguish result categories (e.g. resource vs. namespace vs. pane) so the operator can tell at a glance what selecting a given result will do.
- **FR-007**: System MUST rank results with closer/stronger name matches first within each category, and MUST cap the total number of displayed results to keep the list scannable and responsive.
- **FR-008**: System MUST support full keyboard operation: arrow keys (or equivalent) to move the highlighted result, Enter to select the highlighted result, and Escape to dismiss — the operator must never need a mouse.
- **FR-009**: When a resource result is selected, system MUST open the Inspector for that exact resource, reusing the app's existing resource-inspection navigation behavior.
- **FR-010**: When a namespace result is selected, system MUST set that namespace as the active namespace filter, reusing the app's existing namespace-filter behavior.
- **FR-011**: When a pane result is selected, system MUST navigate to that pane, reusing the app's existing pane-navigation behavior.
- **FR-012**: System MUST scope all resource and namespace results to the currently active cluster context only; it MUST NOT search or return results from any other configured/known context.
- **FR-013**: System MUST NOT match against resource content such as labels, annotations, or spec/status fields — matching is limited to resource name, kind, namespace, and pane display names.
- **FR-014**: System MUST NOT execute any mutating action from the palette — every result, when selected, only navigates the operator's view; it never creates, modifies, or deletes a resource.
- **FR-015**: System MUST show a clear, distinct empty-state message when a query matches no results in any category.
- **FR-016**: System MUST remain usable (namespace and pane search continue to function) when no cluster resources are currently loaded, degrading resource search gracefully rather than erroring.

### Key Entities

- **Palette Query**: The operator's current input string, optionally parsed into an optional `kind` prefix and a `name` fuzzy-match term.
- **Palette Result**: A single selectable entry with a category (resource, namespace, or pane), a display label, and enough identifying information (kind + namespace + name, or pane id, or namespace name) to perform the corresponding navigation action when selected.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: An operator can invoke the palette and land on the Inspector for a specific, known resource in under 5 seconds, without using the mouse.
- **SC-002**: Typing 2-3 characters of a resource, namespace, or pane name surfaces the intended result within the visible (non-scrolled) portion of the results list in at least 90% of cases during manual verification.
- **SC-003**: The palette opens within a perceptibly instant amount of time (no visible delay) after the keyboard shortcut is pressed, regardless of how many resources are currently loaded.
- **SC-004**: Zero backend or protocol changes are required to ship this feature — it is verifiable as a frontend-only change.

## Assumptions

- The palette draws resource results from data already loaded into the frontend store (i.e., resources the operator has already navigated near or that are actively being watched), not from an on-demand cluster-wide fetch — consistent with this being a pure frontend feature with no protocol changes.
- "Tool pane" refers to the app's existing top-level navigable sections (e.g. Settings, Port Forwards, Available Agents) as already defined by the app's navigation/store structure; no new panes are introduced by this feature.
- Fuzzy matching means tolerant substring/subsequence matching (e.g. "my-app" matches "my-application-7f8b"), not full typo-correction or semantic search.
- The palette is a global overlay available from any pane in the three-pane shell, not scoped to a single pane.
- Cross-cluster/cross-context search is explicitly out of scope; only the currently active context's data is searched.
