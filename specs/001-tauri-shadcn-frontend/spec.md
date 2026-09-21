# Feature Specification: Replace Desktop UI with a Responsive, Retro-Themed Web-Technology Frontend

**Feature Branch**: `001-tauri-shadcn-frontend`

**Created**: 2026-09-21

**Status**: Draft

**Input**: User description: "Replace our egui UI and probably a lot of our frontend driver code with a shadcn-based UI with a nice custom retro theme, hosted in Tauri. We've been struggling to get egui to work; if we really want to ship, Tauri is easier and faster, and it lets us make mobile apps and other things in the future. Mobile first, scaling to ultra-wide: the wider the screen, the more dense the information, so on a massive screen you can basically load most of the app at once."

## Context

Fathom is an operator-first Kubernetes desktop client. Today its user interface is an immediate-mode desktop UI (~13.8k lines) that talks to a separate backend (~13.5k lines, plus a ~2.4k-line command/event protocol) over an in-process message channel. The backend and protocol contain no UI-toolkit code, so the backend can be kept and only the presentation layer replaced. This feature swaps the presentation layer for one built on web technology inside a native application shell, keeps every capability the current UI offers, and adds a layout system that works from phone widths up to ultra-wide monitors.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Browse and inspect cluster resources on the new UI (Priority: P1)

An operator launches Fathom, picks a kubeconfig context, optionally narrows to namespaces, navigates the resource tree (Workloads, Config, Network, Storage, Access Control, Custom Resources, Events, Nodes, Namespaces), sees a live-updating list for the chosen kind, searches and sorts it, and selects a row to open an inspector with that resource's details.

**Why this priority**: This is the core loop of the product; without it nothing else matters. It is also the slice that proves the new UI can consume the existing backend end to end.

**Independent Test**: Launch the new app against a test cluster (or the recorded fake backend), switch context, open Pods, type a search query, sort by a column, select a pod, and see its details — with the old UI not involved at all.

**Acceptance Scenarios**:

1. **Given** a valid kubeconfig with two contexts, **When** the operator selects the second context, **Then** all visible lists reset and repopulate with that context's resources, and the active context is shown in the top bar.
2. **Given** the Pods list is open, **When** a pod is created or deleted in the cluster, **Then** the list reflects the change without a manual refresh within the backend's normal sync interval.
3. **Given** a list of 500 resources, **When** the operator types a query, **Then** only matching rows (by name prefix, namespace, case-insensitive) remain, and clearing the query restores the full list.
4. **Given** a row is selected, **When** the inspector opens, **Then** it shows the same detail sections the current UI shows for that kind (metadata, status/conditions, spec summary, labels/annotations, raw object).
5. **Given** namespaces are filtered globally, **When** the operator switches to any namespaced kind, **Then** the same namespace filter applies.

---

### User Story 2 - Adaptive density from phone to ultra-wide (Priority: P1)

The same app adapts to the viewport. On a phone-width screen, one thing is visible at a time: navigation, a list, or an inspector, with clear back navigation. As the screen widens, more panes appear side by side, and tables show more columns. On a very wide screen (e.g. 5120px ultra-wide), the operator can have navigation, several resource lists, an inspector, logs, and cluster metrics visible at once, without opening or closing anything.

**Why this priority**: This is the defining design goal of the rewrite and shapes every screen's layout. Retrofitting it later would mean redoing every view.

**Independent Test**: Open the app at defined width breakpoints (phone, tablet, laptop, desktop, wide, ultra-wide) and verify the number of simultaneously visible panes and table columns increases monotonically, with no horizontal page scroll and no clipped controls at any width.

**Acceptance Scenarios**:

1. **Given** a phone-width viewport (~390px), **When** the operator selects a pod from the list, **Then** the inspector replaces the list full-screen and a back control returns to the list with scroll position preserved.
2. **Given** a laptop-width viewport, **When** the operator selects a pod, **Then** the list and inspector are visible side by side and navigation is available as a collapsible rail.
3. **Given** an ultra-wide viewport, **When** the app loads, **Then** navigation, at least two resource panes, the inspector, and a persistent bottom or side area for logs/events/metrics are visible simultaneously.
4. **Given** any viewport, **When** a table has more columns than fit, **Then** lower-priority columns are hidden (not squeezed unreadably), and the operator can still reach their values in the inspector.
5. **Given** the operator resizes the window across a breakpoint, **Then** the layout reflows without losing the current selection, search text, or scroll position.
6. **Given** a wide or larger viewport, **When** the operator arranges panes (e.g. pins a second resource list), **Then** that arrangement is remembered for the next launch at the same size class.

---

### User Story 3 - Act on resources safely (Priority: P2)

From lists and inspectors, the operator performs the existing mutating actions — delete (single and bulk), scale, restart — each behind a confirmation that names exactly what will change, with success/failure feedback.

**Why this priority**: Parity with current capabilities; the old UI cannot be retired without it.

**Independent Test**: Select three deployments, choose bulk delete, confirm, and verify exactly one delete command per selected deployment reaches the backend and a result notification appears.

**Acceptance Scenarios**:

1. **Given** a deployment is selected, **When** the operator chooses Scale and enters 3, **Then** a confirmation shows the current and new replica counts, and on confirm the scale request is sent once.
2. **Given** multiple rows are selected, **When** the operator chooses Delete, **Then** the confirmation lists every selected resource by namespace/name, and cancelling sends nothing.
3. **Given** an action fails in the backend, **Then** the operator sees an error notification with the backend's message, and the list is unchanged.

---

### User Story 4 - Logs, port forwards, metrics, and GitOps dashboards (Priority: P2)

The operator streams and follows pod/container logs, creates and manages port forwards (which persist across restarts), views the cluster overview and per-workload metrics charts from configured Prometheus, and uses the Flux dashboard (sources, kustomizations, Helm releases).

**Why this priority**: These are the differentiating tools beyond plain resource browsing and must reach parity before the old UI is removed.

**Independent Test**: Each tool can be demoed independently against the fake backend: a log stream renders appended chunks and follows the tail; a port forward created with a typed namespace/pod appears in the list; a metrics chart renders a time series; the Flux dashboard lists all three resource categories.

**Acceptance Scenarios**:

1. **Given** a running pod, **When** the operator opens logs and enables follow, **Then** new lines appear as they arrive and the view stays pinned to the bottom until the operator scrolls up.
2. **Given** a log stream producing thousands of lines per second, **Then** the UI stays responsive (scrolling and typing remain smooth) and older lines are trimmed past a retention limit.
3. **Given** Prometheus is not configured or unreachable, **Then** metric panels show a clear "metrics unavailable" state instead of empty or broken charts.
4. **Given** saved port forwards exist, **When** the app starts, **Then** they are restored and shown with their live status.

---

### User Story 5 - Keyboard-driven operation and command palette (Priority: P3)

The operator can navigate sections, focus search, move through rows, open the inspector, and trigger actions entirely from the keyboard, including a command palette that jumps to any resource kind, context, or action by typing.

**Why this priority**: The current UI has keyboard shortcuts; power users rely on them. The palette is new but inexpensive once the UI is built.

**Independent Test**: With no mouse, switch context, jump to Services, filter, select the third row, and open its inspector.

**Acceptance Scenarios**:

1. **Given** any screen, **When** the operator presses the palette shortcut and types "svc", **Then** Services is offered and Enter navigates to it.
2. **Given** a focused list, **When** the operator presses the down arrow and Enter, **Then** the next row is selected and its inspector opens.

---

### User Story 6 - Settings and integrations (Priority: P3)

The operator configures per-context integrations (Prometheus, Grafana, Flux; Argo stays "not implemented" as today), toggles the MCP server and its auto-start, and sees the MCP endpoint status — all of which persist exactly as they do today.

**Why this priority**: Required for parity but used rarely.

**Independent Test**: Configure a Prometheus URL for a context, restart the app, and verify it is still set and metrics panels use it.

**Acceptance Scenarios**:

1. **Given** existing settings files from the current app, **When** the new app starts, **Then** it reads them unchanged (no migration step, no data loss).
2. **Given** the MCP server is enabled, **Then** it keeps running and serving agent tools regardless of which screen is open.

---

### Edge Cases

- No kubeconfig, an empty kubeconfig, or an unreachable cluster: the app shows an actionable empty/error state rather than a blank screen.
- Context switch while list requests are in flight: stale results from the previous context must never appear in the new context's lists.
- Very large clusters (10k+ pods): lists remain scrollable and searchable without freezing the UI.
- The backend stops or crashes: the UI shows a disconnected state and does not silently show stale data as live.
- Resource deleted while its inspector is open: the inspector shows "no longer exists" rather than stale details.
- Window resized mid-interaction (dialog open, text being typed): dialogs stay usable and input isn't lost.
- Touch-only devices: every hover-only affordance (tooltips, row actions) has a tap-accessible equivalent.
- High-DPI and 200% OS scaling: text stays crisp and the density rules still apply to the logical width.
- User prefers reduced motion: retro effects that animate (e.g. flicker, scanline motion) are disabled.

## Requirements *(mandatory)*

### Functional Requirements

**Parity**

- **FR-001**: The new UI MUST provide every resource kind currently browsable (pods, deployments, daemonsets, statefulsets, replicasets, replication controllers, jobs, cronjobs, nodes, namespaces, events, services, endpoints, ingresses, ingress classes, network policies, persistent volumes, persistent volume claims, storage classes, roles, role bindings, cluster roles, cluster role bindings, config maps, secrets, resource quotas, limit ranges, horizontal pod autoscalers, pod disruption budgets, priority classes, runtime classes, leases, mutating and validating webhook configurations, and Flux custom resources), each with list and detail views.
- **FR-002**: The new UI MUST provide every mutating action currently available (delete, bulk delete, scale, restart where applicable, create/stop port forward), each gated by an explicit confirmation.
- **FR-003**: The new UI MUST provide overview dashboard, workload metrics charts, log streaming with follow, port-forward management, integrations settings, MCP settings, and a status bar with backend notifications.
- **FR-004**: The system MUST keep the existing backend behavior, the agent-facing MCP server, and the existing on-disk settings format, so the current app's saved state works unchanged.
- **FR-005**: The system MUST keep the headless (MCP-only, no window) run mode.

**Responsive density**

- **FR-006**: The layout MUST be designed mobile-first and MUST define at least six width classes (phone, tablet, laptop, desktop, wide, ultra-wide). The number of simultaneously visible panes MUST never decrease as width increases.
- **FR-007**: At phone width, the UI MUST show one primary pane at a time (navigation, list, inspector, or tool) with explicit back navigation and preserved state.
- **FR-008**: At each wider class, the UI MUST add visible information rather than only enlarging it: side-by-side list and inspector, a persistent navigation rail/tree, more table columns, and at the widest classes, multiple concurrent resource panes plus persistent logs/events/metrics areas.
- **FR-009**: Tables MUST assign every column a priority; columns MUST appear or disappear by priority as width changes, never shrink below readable width, and hidden values MUST remain reachable in the inspector.
- **FR-010**: The page MUST never scroll horizontally at any supported width; only designated regions (e.g. a log line or raw object view) may scroll horizontally.
- **FR-011**: On wide and larger classes, operators MUST be able to pin, rearrange, and resize panes, and the arrangement MUST persist per width class.
- **FR-012**: All interactive controls MUST be usable by touch at phone and tablet widths (touch targets at least 44×44 CSS px) and by mouse and keyboard at all widths.

**Live data and responsiveness**

- **FR-013**: Lists MUST update live from backend events without full re-renders that lose selection, scroll, or search state.
- **FR-014**: The UI MUST remain interactive (input handled within a frame budget perceptible as instant) while receiving continuous list updates and high-volume log streams.
- **FR-015**: The UI MUST NOT redraw continuously when nothing has changed (idle CPU use must be near zero).
- **FR-016**: Events from a previous context or a superseded request MUST be discarded and never shown.
- **FR-017**: The UI MUST show clear loading, empty, error, disconnected, and "metrics unavailable" states for every data-bearing pane.

**Theme and interaction**

- **FR-018**: The UI MUST use a single custom retro visual theme applied consistently via shared design tokens (colors, type, spacing, borders, effects), in both a dark (default) and a light variant. Type rules are defined by FR-021.
- **FR-019**: The retro theme MUST meet WCAG 2.2 AA contrast for text and essential UI, and all decorative effects MUST be disableable and MUST respect the OS reduced-motion preference.
- **FR-020**: The UI MUST support full keyboard operation and a command palette covering navigation to every section, context switching, and common actions.
- **FR-021**: The retro theme MUST follow a "neon-noir terminal" direction. The dark variant uses neon cyan (default accent) or magenta (selectable accent), with violet structure, on a deep abyssal navy. Data (tables, identifiers, logs, YAML, metric legends) is set in monospace. Headings and chrome use an angular, blocky display face. The mark is the kanji 尋 (*hiro*, the Japanese unit for a fathom) in brush lettering. The theme has square corners, box-drawing-style borders and dividers, and a block-cursor/inverse-video treatment for selection and focus. Glitch (RGB split), scanline, grain/wear, and circuit-trace details are decorative only, limited to shell chrome, transitions, and empty/error states (never table rows or log lines), and disableable (FR-019). The light variant is a legible "blueprint" counterpart (dark ink on pale paper) that keeps the same type and structure.

**Quality and delivery**

- **FR-022**: Every user story MUST have automated end-to-end tests that run the UI against a scripted fake backend (replacing today's UI test harness, which injects events and asserts emitted commands), plus unit tests for UI logic (search, sort, column priority, layout class selection).
- **FR-023**: End-to-end tests MUST run at a phone width and at least one wide width for each P1 story.
- **FR-024**: The desktop app MUST ship for the platforms currently released (Linux x86_64 and arm64, Windows x86_64, macOS x86_64 and arm64) from the existing CI/release flow, and MUST be validated on the constitution's Tier 1 platforms: Ubuntu 26.04 LTS, Windows 11, and macOS 26 Tahoe.
- **FR-025**: The old UI MUST remain buildable until the new UI reaches parity (FR-001–FR-005), after which the old UI code, its dependencies, and its tests MUST be removed.
- **FR-026**: The command and event contract between UI and backend MUST be defined once and shared by both sides, so a change on one side that breaks the other fails the build or tests rather than failing at runtime.

### Key Entities

- **Command**: A request from the UI to the backend (refresh a kind, select a resource, delete, scale, restart, start/stop logs or port forward, change context/namespaces, configure integrations). ~137 kinds today.
- **Event Envelope**: A backend message to the UI carrying a sequence number, route (which section/scope it belongs to), correlation (which request it answers), and one event: system, resource list, resource detail, or stream chunk. ~90 kinds today.
- **Route / Section**: The navigational location (resource kind + scope) that events are addressed to and that panes display.
- **Pane**: A visible region showing one route or tool (list, inspector, logs, metrics, dashboard). It has a width-class-dependent placement and optional pinning.
- **Layout Profile**: The persisted arrangement of panes for a given width class.
- **Theme Tokens**: The named set of colors, typography, spacing, borders, and effect switches that define the retro look in dark and light variants.
- **Settings**: Existing persisted user data (contexts' integrations, credentials, saved port forwards, MCP auto-start, navigation expansion state), which must remain format-compatible.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: 100% of the resource kinds, actions, and tools listed in FR-001–FR-003 are available in the new UI, and the old UI is removed.
- **SC-002**: At ultra-wide width (≥ 3840 logical px), an operator can see at least 5 distinct panes (navigation, 2+ resource lists, inspector, logs/metrics) at once without opening or closing anything. At phone width (390px) every screen is fully usable with no horizontal page scroll.
- **SC-003**: From launch to a populated pods list for a reachable cluster takes no longer than it does in the current app (baseline measured before the switch), and under 3 seconds on a typical developer laptop.
- **SC-004**: With a 10,000-pod list receiving continuous updates, search results update and scrolling stays smooth (no visible stutter; input response under 100 ms).
- **SC-005**: When the app is idle (no data changes) for 60 seconds, CPU use is below 2% of one core (today's UI redraws continuously).
- **SC-006**: All P1 and P2 user stories pass automated end-to-end tests at phone and wide widths in CI on every change.
- **SC-007**: Median time to build a new resource-kind screen (list + inspector) drops to under half a day, measured on the first three kinds built after the core loop exists.
- **SC-008**: Theme contrast checks pass WCAG 2.2 AA for 100% of text tokens in both variants.
- **SC-009**: Existing users upgrade with zero manual steps: all saved port forwards, integrations, and MCP settings are present after first launch.

## Clarifications

### Session 2026-09-21

- Q: Which retro visual direction? → A: Phosphor CRT terminal (green/amber on near-black, monospace, square corners).
- Q: (Revised, same day, after a branding review) Which visual direction? → A: Neon-noir terminal. Keep the terminal structure, and use a neon cyan/magenta/violet palette on abyssal navy, an angular display font, the 尋 kanji mark, and glitch/wear effects only on chrome. This supersedes the phosphor answer.

## Assumptions

- The existing backend (cluster access, sync, persistence, MCP server) stays and is reused as-is; only small, contract-level changes (e.g. making all commands/events serializable, adding a UI wake-up/notify path) are in scope.
- Mobile store apps (phone/tablet builds) are **out of scope** for this feature; this feature makes the UI mobile-first and layout-ready so a later feature can target mobile. How the backend would run on mobile (on-device vs. remote) is deferred to that feature.
- A browser-only/web-hosted version is out of scope, though nothing in the UI should preclude it.
- The UI is desktop-local: one user, one machine, no authentication beyond the user's kubeconfig.
- The "Applications" and "Helm" views described in the product MVP doc but not built in the current UI are out of scope; this feature targets parity with what the current UI actually implements.
- Argo integration remains "not implemented", as it is today.
- The migration is incremental: the old and new UIs can coexist in the repository until parity is reached (FR-025).
- The operator is a power user on a modern OS with a current system web engine.
