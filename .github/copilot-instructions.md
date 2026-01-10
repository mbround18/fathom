# Copilot Instructions for Fathom

## Repository Overview

**Type**: Desktop Kubernetes dashboard application  
**Primary Language**: Rust (edition 2024)  
**Architecture**: Multi-threaded GUI with async backend (eframe UI thread + tokio runtime thread)  
**Target Audience**: Experienced Kubernetes operators and platform engineers

## Core Architectural Principles

### Thread Model

- **UI thread**: Runs eframe/egui event loop. Never blocks. Never awaits async code.
- **Backend thread**: Owns tokio runtime. Handles all Kubernetes API calls and async operations.
- **Communication**: Crossbeam unbounded channels for UI↔backend message passing.
- **State**: UI state lives in `FathomApp`. Backend state lives in `BackendState`. No shared mutable state except explicitly synchronized (e.g., `Arc<RwLock<Vec<String>>>` for namespace filter).

### Protocol Design

- All commands from UI to backend use `BackendCmd` enum (defined in `protocol/v1/cmd.rs`).
- All events from backend to UI use `BackendEvent` enum wrapped in `BackendEventEnvelope` (defined in `protocol/v1/event.rs`).
- Protocol types in `protocol/` are pure data structures with no behavior. Use `#[derive(Debug, Clone)]`.
- Backend emits events via `events::emit()` or `events::notify_*()` helpers.
- UI consumes events via `check_mailbox()` pattern (drain channel, apply updates).

### Module Organization

- `src/app/`: UI rendering logic (egui). Each `ui_*.rs` file is a render pass (top bar, left nav, center panel, right panel, status bar, modals).
- `src/backend/`: Async Kubernetes operations. Each resource type gets its own module (e.g., `pods.rs`, `deployments.rs`).
- `src/protocol/`: Shared data types for UI↔backend communication. Version under `v1/`.
- `src/backend/state.rs`: Backend-owned state and helper functions for task lifecycle management.

### Code Reuse Strategy

- Prefer generic implementations over copy-paste. Example: `WorkloadSummary` and `WorkloadDetails` types are shared across Deployments, DaemonSets, StatefulSets, ReplicaSets, and ReplicationControllers.
- UI rendering for workloads uses shared helpers (e.g., `render_workload_grid()`, `render_workload_details_panel()`).
- Backend handlers follow consistent patterns: `refresh_*`, `fetch_*_details`, `scale_*`, `restart_*`, `delete_*`.

## Coding Standards

### Rust Style

- Follow Rust 2024 edition conventions.
- Use `rustfmt` defaults (no custom configuration).
- Prefer explicit types in function signatures. Type inference is fine for locals.
- Use `?` operator for error propagation. Return `anyhow::Result<T>` for fallible operations.
- Avoid `.unwrap()` in production code paths. Use `.expect("reason")` only when invariant is guaranteed or for initialization.

- **Do not run `cargo build` or `cargo check` with `--release` unless explicitly requested.** Always use the default debug profile for development and testing unless the user specifically asks for a release build.

### Naming Conventions

- Functions: `snake_case`. Async functions: no special prefix.
- Types: `PascalCase`. Protocol types: suffixed with purpose (e.g., `PodSummary`, `PodDetails`, `PodMetricsSummary`).
- Constants: `SCREAMING_SNAKE_CASE` (e.g., `MAX_ITEMS`).
- Backend state fields: descriptive and scoped (e.g., `pod_watch_task`, `namespace_filter_shared`).

### Error Handling

- **Best-effort philosophy**: Operations that might fail due to RBAC, CRD absence, or transient API errors should degrade gracefully.
- Log errors at `tracing::warn!()` level for best-effort failures. Use `tracing::error!()` for unexpected failures.
- Emit user-visible notifications via `events::notify_error()` for actionable failures only.
- Never panic in response to external input or Kubernetes API responses.
- Annotation writes (e.g., `fathom/interactions`) must be fire-and-forget: log failure but continue operation.

### Async and Concurrency

- All Kubernetes API calls are `async` and run on the backend tokio runtime.
- Use `tokio::spawn()` for background tasks. Store `JoinHandle<()>` in `BackendState` for lifecycle management.
- Always abort background tasks on context switch, disconnect, or shutdown via `state::abort_task()`.
- Use `Arc<RwLock<T>>` sparingly. Prefer message-passing over shared state.
- Never call `.block_on()` inside an async context.

### Kubernetes API Patterns

- Use `kube::Api<T>` for resource operations. Generic bounds: `T: Clone + Debug + DeserializeOwned + Serialize`.
- For metadata access: add `T: kube::ResourceExt` bound.
- Fetch lists with `ListParams::default()` or scoped by namespace.
- Use `PartialObjectMetaExt` trait for metadata-only operations where possible.
- Owner references: parse via `obj.metadata.owner_references`. Resolve indirect ownership (e.g., ReplicaSet→Deployment) with best-effort GET.
- Metrics API: probe availability with `Api::<PodMetrics>::list(&api, &Default::default()).await` at startup.

### Data Annotation Strategy

- Use `fathom/interactions` annotation key for action audit logs.
- Format: JSON `{ "v": 1, "items": [...] }` with ring buffer (max 50 items).
- Each item: `{ "action": "delete|restart|scale", "timestamp": "ISO8601", "context": "string", "user": "string", "details": {...} }`.
- User is best-effort from kubeconfig `contexts[].context.user` field (may be empty).
- Annotation writes must not block main operation path. Use `let _ =` for fire-and-forget.

## UI/UX Principles

### Responsiveness

- UI must never block. Long operations happen in backend thread; UI polls mailbox.
- Use `ctx.request_repaint()` to keep UI responsive during active event streams.
- Show pending/loading states (e.g., "Fetching metrics…") rather than stale data.

### Interlinking

- Resource relationships are navigable: pod→owner workload, namespace labels toggle filters.
- Clickable elements: use `ui.link()` or `ui.selectable_label().clicked()` for navigation.
- Navigation actions may require switching `active_section` and sending backend commands (e.g., `SelectDeployment`).

### Metrics Display

- Show metrics at the top of details panels when available.
- Auto-fetch metrics on selection/tab open to ensure freshness.
- Use clear messaging: "Fetching metrics…" (pending), "(metrics not loaded)" (unavailable), or actual values.

### Form Layout

- Keep forms visually symmetric: align labels and inputs on a consistent grid.
- Use one pattern per form: either inline (`Label: <input>`) with padded spacing so inputs start at the same x-position, or stacked (`Label` on its own line with the input directly beneath). Apply the chosen pattern uniformly.
- Keep label widths consistent within a form; add padding/margins so fields line up.
- Avoid mixing inline and stacked patterns in the same form and avoid ragged alignment.

### Modal Actions

- Destructive actions (delete, restart) require confirmation modal.
- Modal content: resource name, namespace, action description, Cancel/Confirm buttons.
- Execute action on confirm, refresh resource list afterward.

## Testing Philosophy

### Current State

- No automated tests yet. Code is production-ready but test coverage is planned.

### Future Testing Expectations

- **Unit tests**: Pure functions (e.g., protocol type conversions, filtering, sorting).
- **Integration tests**: Mock `kube::Client` with `tower_test::mock` for backend handlers.
- **UI tests**: Snapshot testing for egui rendering (if tooling becomes available).
- **Edge cases**: RBAC failures, missing CRDs, malformed kubeconfig, empty clusters.

## Security and Secrets

### Kubeconfig

- Read from default kubeconfig path (`~/.kube/config` or `KUBECONFIG` env).
- Use `kube::Config::infer()` for standard discovery (kubeconfig → in-cluster).
- Never log token values, client certificates, or keys. Log context names only.

### Kubernetes RBAC

- Assume restrictive RBAC. Gracefully handle 403 Forbidden responses.
- Do not fail entire operations due to missing permissions on annotation writes or metrics queries.

### Annotation Data

- Interaction logs may include kubeconfig user names but no tokens or secrets.
- Keep annotation payload small (ring buffer, compact JSON) to avoid API server limits.

## Performance and Scalability

### Resource Limits

- Expect clusters with 100s of namespaces and 1000s of pods.
- Use Kubernetes watch APIs for pods (background `watch()` stream via `watcher()` helper).
- Refresh deployments/workloads on-demand or via periodic background sync (avoid constant polling).
- Cache details in UI state; refresh only on user action or watch event.

### Memory Management

- Bounded buffers: ring buffers for logs (max items), limited backscroll for pod logs.
- Drop background task handles on disconnect to free memory.

## Documentation

### Code Comments

- Document non-obvious design decisions (e.g., "ReplicaSet → Deployment resolution requires extra GET").
- Protocol types: add doc comments for field purpose if not self-evident.
- Module-level docs: describe module purpose at top of each `mod.rs`.

### Inline Documentation

- Use `///` for public API docs.
- Use `//` for implementation notes.
- Avoid restating the obvious. Comment _why_, not _what_.

## What Copilot Should Avoid

- **Do not** use `.unwrap()` on Kubernetes API results or user input.
- **Do not** spawn threads directly; use `tokio::spawn()` for backend tasks.
- **Do not** introduce new global state or `lazy_static!` without strong justification.
- **Do not** add heavyweight dependencies (e.g., full ORM, large frameworks) without discussion.
- **Do not** break the UI/backend thread boundary (no `await` in UI, no egui in backend).
- **Do not** assume Kubernetes resources exist (CRDs, namespaces, metrics API may be absent).
- **Do not** add logging/tracing macros in hot loops (e.g., UI render pass).

## Preferred Output Formats

### Configuration

- Kubernetes YAML: prefer inline rather than separate files unless structurally necessary.
- Application config: JSON (for future `config.json` persistence).

### Logging

- Use `tracing` crate (`info!`, `warn!`, `error!`, `debug!`).
- Prefer structured fields: `tracing::info!(namespace = %ns, pod = %name, "Deleted pod")`.

### Documentation

- Markdown for README, design docs, and this file.
- Inline doc comments for Rust types and functions.

## Safe Assumptions

Copilot may assume:

- User has `kubectl` installed and a valid kubeconfig.
- Kubernetes cluster is reachable and API server is responsive (but may lack permissions).
- Standard Kubernetes resources (Pods, Deployments, etc.) exist in the cluster.
- UI runs on Windows, macOS, or Linux (standard `eframe` target platforms).

Copilot must NOT assume:

- Specific Kubernetes version (use conservative API usage compatible with 1.25+).
- Metrics API availability (probe and degrade gracefully).
- Write permissions for annotations (best-effort only).
- Cluster size or resource count (scale to thousands of resources).
- User's preferred kubeconfig context (always respect user selection).

## Shutdown and Lifecycle

- Application must handle Ctrl+C (`ctrlc` crate) and window close cleanly.
- On shutdown: send `BackendCmd::Shutdown`, abort all background tasks, join backend thread.
- Backend cleanup: abort pod watcher, namespace sync, log streams via `cleanup_all_tasks()`.
- No resource leaks: all `JoinHandle` and `Child` process handles must be tracked and cleaned up.
