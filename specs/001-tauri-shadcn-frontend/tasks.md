---
description: "Task list for replacing the egui UI with Tauri v2 + React/shadcn (neon-noir terminal theme)"
---

# Tasks: Replace Desktop UI with a Responsive, Retro-Themed Web-Technology Frontend

**Input**: Design documents from `specs/001-tauri-shadcn-frontend/`

**Prerequisites**:
- [plan.md](./plan.md)
- [spec.md](./spec.md)
- [research.md](./research.md)
- [data-model.md](./data-model.md)
- [contracts/ipc.md](./contracts/ipc.md)
- [contracts/layout.md](./contracts/layout.md)
- [contracts/theme.md](./contracts/theme.md)
- [quickstart.md](./quickstart.md)

**Tests**: These are required. FR-022/FR-023 mandate Vitest unit tests plus Playwright E2E per user story, run at phone and wide widths, against the fake transport. Test tasks come before the implementation tasks they cover, and they should fail first.

**Organization**: Phases 1–2 are shared plumbing. Phases 3–8 map to user stories US1–US6 in priority order. Phase 9 covers cutover and polish.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: can run in parallel (different files, no dependency on an incomplete task)
- **[USn]**: the user story in spec.md that the task belongs to

## Path conventions (from plan.md)

- Rust core library: `src/` (crate `fathom`)
- Tauri shell crate: `ui/src-tauri/` (crate `fathom-desktop`)
- Frontend: `ui/src/`; unit tests in `ui/tests/unit/`; E2E tests in `ui/tests/e2e/`; fixtures in `ui/tests/fixtures/`
- Dagger CI: `pipeline/src/`; GitHub Actions: `.github/workflows/`

---

## Phase 1: Setup (Shared Infrastructure)

**Purpose**: Scaffold the frontend package and the Tauri crate, and make the old UI optional.

- [X] T001 Add a cargo feature `egui-ui` (in `default`) to root `Cargo.toml`, and make `eframe`, `egui`, and `egui_plot` (and any other egui-only normal deps) `optional = true` and enabled by `egui-ui`. Leave `egui_kittest` as a plain dev-dependency, because cargo does not allow optional dev-deps; the `required-features` on the GUI tests gates it. Add `required-features = ["egui-ui"]` to the `fathom` `[[bin]]` and to the `[[test]]` entries for `tests/gui_*.rs`. Verify with `cargo check --no-default-features --lib` and `cargo test` (the default still builds egui).
- [X] T002 Gate `pub mod app;` behind `#[cfg(feature = "egui-ui")]` in `src/lib.rs`, and fix any non-app module that imports from `crate::app`. `cargo check --no-default-features --lib` must pass with zero egui symbols.
- [X] T003 Scaffold the frontend in `ui/`:
  - Run `pnpm create vite ui --template react-ts`. Set `"packageManager": "pnpm@9"` and the scripts `dev`, `dev:fake` (`VITE_TRANSPORT=fake vite`), `build`, `typecheck` (`tsc -b --noEmit`), `lint`, `test` (`vitest run`), `e2e` (`playwright test`), and `tauri` in `ui/package.json`.
  - Enable TS `strict`, `noUncheckedIndexedAccess`, and the path alias `@/* → src/*` in `ui/tsconfig.json`.
- [X] T004 Add Tauri v2 to `ui/`:
  - `pnpm -C ui add -D @tauri-apps/cli@^2` and `pnpm -C ui add @tauri-apps/api@^2`.
  - Run `pnpm -C ui tauri init`, with the app name `Fathom`, identifier `dev.fathom.app`, dev URL `http://localhost:5173`, and frontend dist `../dist`. That creates `ui/src-tauri/`.
  - Set `devUrl`, `frontendDist`, `beforeDevCommand: "pnpm dev"`, `beforeBuildCommand: "pnpm build"`, window `minWidth: 360`, and title `Fathom` in `ui/src-tauri/tauri.conf.json`.
- [X] T005 Make `ui/src-tauri` a workspace crate:
  - In `ui/src-tauri/Cargo.toml`, name it `fathom-desktop`, set `edition = "2024"`, `[lib] name = "fathom_desktop"`, and `crate-type = ["staticlib", "cdylib", "rlib"]`.
  - Add the deps `fathom = { path = "../..", default-features = false }`, `tauri = "2"`, `serde`, `serde_json`, `crossbeam-channel`, `tracing`, and `ctrlc`.
  - Add `"ui/src-tauri"` to `[workspace] members` in root `Cargo.toml`.
- [X] T006 [P] Install and configure Tailwind v4 and shadcn:
  - `pnpm -C ui add tailwindcss @tailwindcss/vite`, and register the plugin in `ui/vite.config.ts`.
  - Run `pnpm -C ui dlx shadcn@latest init`. Choose Radix, CSS variables, and base color neutral; the tokens are replaced in T020. That creates `ui/components.json` and `ui/src/lib/utils.ts`.
- [X] T007 [P] Add the runtime deps to `ui/package.json`:
  - `zustand`, `@tanstack/react-table`, `@tanstack/react-virtual`, `react-resizable-panels`, `cmdk`, `recharts`, `sonner`, `@fontsource/jetbrains-mono`, `@fontsource/chakra-petch`, `@fontsource/yuji-boku` (fallback `@fontsource/noto-serif-jp` if unavailable).
  - Add the shadcn components into `ui/src/components/ui/` with `pnpm -C ui dlx shadcn@latest add button input dialog alert-dialog dropdown-menu select tabs tooltip scroll-area resizable command sheet badge separator skeleton sonner checkbox switch table`.
- [X] T008 [P] Configure testing:
  - `pnpm -C ui add -D vitest @testing-library/react @testing-library/user-event @testing-library/jest-dom jsdom @playwright/test`.
  - Create `ui/vitest.config.ts` with the jsdom env and setup file `ui/tests/unit/setup.ts`.
  - Create `ui/playwright.config.ts` with:
    - `webServer: pnpm dev:fake`.
    - Projects `phone` (390×844, hasTouch), `tablet` (820×1180), `laptop` (1280×800), `desktop` (1600×900), `wide` (1920×1080), `ultra` (2560×1440), and `ultra-plus` (5120×1440), each for `chromium` and `webkit` (see contracts/layout.md for the test hooks).
- [X] T009 [P] Configure ESLint (typescript-eslint, react-hooks, jsx-a11y) and Prettier in `ui/eslint.config.js` and `ui/.prettierrc`. Add `ui/node_modules`, `ui/dist`, `ui/src-tauri/target`, `ui/test-results`, and `ui/playwright-report` to `.gitignore`.
- [X] T010 [P] Generate app icons with `pnpm -C ui tauri icon ../assets/icon.png` into `ui/src-tauri/icons/`, and reference them in `ui/src-tauri/tauri.conf.json` under `bundle.icon`.

**Checkpoint**: Both of these work: `pnpm -C ui tauri dev` opens a blank window, and `cargo run` still opens the egui UI.

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: The protocol contract, the bridge, transports, the store, the theme, the layout shell, and CI. Every user story depends on these.

**⚠️ CRITICAL**: No user story work can start until this phase is complete.

### Protocol serialization (FR-026, contracts/ipc.md)

- [X] T011 Add `ts-rs = { version = "12", features = ["serde-compat", "no-serde-warnings"] }` as a dependency in root `Cargo.toml`. It is non-optional, because the lib must export the types without egui.
- [X] T012 Derive `Serialize, Deserialize, TS` with `#[ts(export, export_to = "../ui/src/protocol/generated/")]` on the ID and route types:
  - `RequestId`, `StreamId`, and `Correlation` in `src/protocol/v1/ids.rs`.
  - `SectionKey`, `ScopeKind`, and `Route` in `src/protocol/v1/route.rs`.
  - Export `u64` IDs as `number` using `#[ts(type = "number")]`.
- [X] T013 Derive `Serialize, Deserialize, TS` on `BackendCmdV1` and every payload type it references in `src/protocol/v1/cmd.rs`.
  - Use `#[serde(tag = "type", content = "data")]`.
  - Follow every referenced type into the per-kind modules `src/protocol/v1/*.rs` (including `cronjobs.rs` and `jobs.rs`, which currently lack serde) and add the derives there too.
- [X] T014 Derive `Serialize, Deserialize, TS` on `BackendEventEnvelopeV1`, `BackendEventV1`, `SystemEvent`, `ResourceListEvent`, `ResourceReadEvent`, and `StreamEvent` in `src/protocol/v1/event.rs`.
  - Every enum is adjacently tagged: `#[serde(tag = "type", content = "data")]`.
  - Follow every referenced type into `src/protocol/v1/*.rs` and `src/sections/`.
- [X] T015 Add `UiConfig`, `ThemePrefs`, `LayoutProfile`, `SlotConfig`, `DockConfig`, and `WidthClass` exactly as specified in data-model.md §2, with `schema: 1`, in a new file `src/protocol/v1/ui_config.rs`. Keep these constraints verbatim:
  - `mode: "dark" | "light"` (default `"dark"`)
  - `accent: "cyan" | "magenta"` (default `"cyan"`)
  - `effects: "on" | "off"` (default `"on"`)
  - `glitch` defaults to `false`
- [X] T016 Add the command variants to `BackendCmdV1` in `src/protocol/v1/cmd.rs`:
  - `LoadUiConfig`
  - `SaveUiConfig { ui: UiConfig, expanded_groups: Vec<String> }`
  - `SetMcpAutoStart { enabled: bool }`
- [X] T017 Add `SystemEvent::UiConfigLoaded { ui: Option<UiConfig>, expanded_groups: Vec<String>, mcp_auto_start: bool }` in `src/protocol/v1/event.rs`.
- [X] T018 Handle the new commands in `src/backend/cmd.rs`, using `src/backend/persistence.rs`:
  - **Read** `config.json → ui`, `navigation.expanded_groups` (default `["Overview", "Workloads"]`), and `auto_start.mcp_http`.
  - **Write** `ui` and `navigation.expanded_groups` without touching other keys. `SetMcpAutoStart` calls `save_mcp_auto_start`.
  - **Validate** the `ui` blob. If it is invalid, or `schema != 1`, move it to `ui_invalid` and return `ui: None`.
- [X] T019 [P] Add serde round-trip tests in a new file `tests/protocol_serde.rs` (a test, not gated on `egui-ui`):
  - Construct each `BackendCmdV1` variant and each event variant with sample data.
  - Assert that `serde_json::to_value` → `from_value` → `to_value` is stable.
  - Snapshot the JSON of `RefreshPods`, `Shutdown`, and one `ResourceList` event, to pin the adjacently tagged shape (`{"type": …, "data": …}`).
- [X] T020 [P] Add a unit test for the new persistence path in `tests/ui_config_persistence.rs`:
  - Point `HOME` at a temp dir. `SaveUiConfig` followed by `LoadUiConfig` preserves `ui`, keeps unrelated keys (`auto_start`, other top-level keys) byte-identical, and a corrupt `ui` blob ends up in `ui_invalid`.
- [X] T021 Add a `cargo test` target `export_bindings` in `tests/export_bindings.rs`. It calls `TS::export_all` for the root types (`BackendCmdV1`, `BackendEventEnvelopeV1`, `UiConfig`) into `ui/src/protocol/generated/`. Then generate the files and commit them, along with `ui/src/protocol/generated/index.ts`, which re-exports everything.

### Bridge (research R1, contracts/ipc.md)

- [X] T022 Implement the bridge in a new file `ui/src-tauri/src/bridge.rs`:
  - `BridgeState { cmd_tx: Sender<BackendCmd>, subscriber: Mutex<Option<Channel<EventBatch>>>, buffer: Mutex<VecDeque<BackendEventEnvelope>> }`.
  - `EventBatch { envelopes, dropped: u32 }`, with `Serialize` + `TS` exported to `ui/src/protocol/generated/`.
  - `#[tauri::command] send_cmd(cmd)`, which returns `Err(BridgeError::BackendGone)` if the send fails.
  - `#[tauri::command] subscribe_events(on_batch: Channel<EventBatch>) -> SubscriptionInfo { protocolVersion: 1, backendStarted }`. A new subscription replaces the old one and flushes the buffer first.
- [X] T023 Implement the pump thread in `ui/src-tauri/src/bridge.rs`:
  - Block on `gui_rx.recv()`, then drain with `try_recv` until 256 envelopes or 16 ms, and send one batch.
  - With no subscriber, buffer up to 10,000. On overflow, evict the oldest `ResourceList` first, then `Stream`, never `System`. Count the evictions in `dropped`.
  - If `Channel::send` returns an error, detach the subscriber and resume buffering.
- [X] T024 [P] Add unit tests in `ui/src-tauri/src/bridge.rs` (`#[cfg(test)]`) for the batch limits (256 / 16 ms), the eviction order, `dropped` accounting, and resubscribe flushing the buffer. Factor the pump so it is testable, with an injectable sink trait.
- [X] T025 Implement `run()` in `ui/src-tauri/src/lib.rs`, with `#[cfg_attr(mobile, tauri::mobile_entry_point)]`:
  - Create the crossbeam channels.
  - Spawn the backend thread running `fathom::backend::init`, mirroring `src/main.rs`.
  - `.manage(BridgeState)` and `invoke_handler(generate_handler![send_cmd, subscribe_events])`.
  - Start the pump.
  - On `WindowEvent::CloseRequested` and on ctrlc, send `BackendCmd::Shutdown`, wait up to 500 ms, and exit.
- [X] T026 Implement `ui/src-tauri/src/main.rs`:
  - Parse `--only-mcp` exactly as `src/main.rs` does, and run the headless MCP path **before** building Tauri.
  - Otherwise call `fathom_desktop::run()`.
  - Keep `#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]`.
- [X] T027 Restrict the webview to `core:default` plus the two app commands (`allow-send-cmd`, `allow-subscribe-events`) in `ui/src-tauri/capabilities/default.json`. Declare those commands in `ui/src-tauri/build.rs` with `tauri_build::try_build(Attributes::new().app_manifest(AppManifest::new().commands(&["send_cmd", "subscribe_events"])))`.

### Transports (contracts/ipc.md)

- [X] T028 Define the `Transport` interface (`send(cmd)`, `subscribe(onBatch) → SubscriptionInfo`) and a `getTransport()` that picks one from `import.meta.env.VITE_TRANSPORT` (`"tauri"` by default, or `"fake"`), in `ui/src/protocol/transport.ts`.
- [X] T029 [P] Implement `TauriTransport` with `invoke('send_cmd', { cmd })` and `new Channel<EventBatch>()` passed to `invoke('subscribe_events', { onBatch })`, in `ui/src/protocol/tauri-transport.ts`.
- [X] T030 [P] Implement `FakeTransport` in `ui/src/protocol/fake-transport.ts`. It:
  - records every sent command in `window.__fathom.sent`;
  - exposes `window.__fathom.emit(batch)` and `window.__fathom.loadScenario(name)` for Playwright;
  - auto-replies to `LoadUiConfig`, context listing, and `Refresh*` commands from fixture files in `ui/tests/fixtures/`.
- [X] T031 [P] Create the fixture scenarios as `EventBatch` JSON under `ui/tests/fixtures/`. Each one must type-check against the generated types:
  - `two-contexts.json` (contexts `kind-a` and `kind-b`)
  - `pods-500.json`
  - `pods-10k.json` (generated by the script `ui/tests/fixtures/gen-pods.ts`)
  - `deployments.json`
  - `ui-config-default.json`
- [X] T032 [P] Add a Vitest contract test in `ui/tests/unit/fixtures.contract.test.ts`. It imports each fixture, checks it with `satisfies EventBatch`, and asserts that every envelope has `version === 1`.

### Store (data-model.md §3)

- [X] T033 Create the Zustand store skeleton in `ui/src/store/index.ts`:
  - The slices `conn`, `session`, `lists`, `details`, `streams`, `metrics`, `flux`, `panes`, `layout`, `selection`, `pending`, and `ui`.
  - Key helpers in `ui/src/store/keys.ts`:
    - `listKey = "${context}|${section}|${scope}|${ns}"`
    - `detailKey = "${context}|${kind}|${ns}|${name}"`
- [X] T034 Write reducer tests first, in `ui/tests/unit/ingest.test.ts`. They cover these cases:
  - envelopes for a non-active context are dropped, except System ones;
  - a `Request` correlation id older than `latestRequest` is dropped (`Correlation` is the tagged enum `None | Request(id) | Stream(id)`);
  - unchanged rows (`uid` + `resourceVersion`) keep object identity;
  - one batch produces exactly one store notification;
  - `version !== 1` sets the fatal state;
  - `dropped > 0` is reflected in `conn.dropped`.
- [X] T035 Implement `ingest(batch)` in `ui/src/store/ingest.ts`, with one `set()` per batch following the five rules in data-model.md §3. Dispatch per event arm to handlers in `ui/src/store/handlers/{system,list,read,stream}.ts`.
- [X] T036 Implement a `RingBuffer<T>` with capacity 10,000 and a `truncated` counter in `ui/src/store/ring-buffer.ts`, with tests in `ui/tests/unit/ring-buffer.test.ts`.
- [X] T037 Implement `useBackend()` in `ui/src/protocol/use-backend.ts`. On mount it subscribes via the transport, routes batches into `ingest`, sends `LoadUiConfig` and the initial context/refresh commands, and sets `conn.status` (`starting → ready`, or `backend-gone` on a `send` rejection).

### Theme (contracts/theme.md, FR-018/019/021)

- [X] T038 Write `ui/src/theme/tokens.css` in OKLCH, following contracts/theme.md:
  - `.dark` (the default, neon-noir) with `html[data-accent="cyan"|"magenta"]` variants, and `:root` for the light "blueprint" variant.
  - All the shadcn variables plus `--accent-neon`, `--accent-2`, `--glow`, `--scanline-opacity`, `--grain-opacity`, `--status-ok/warn/err`, and `--chart-1..5`, with `--radius: 0`.
  - An `@theme inline` mapping for Tailwind v4, exposing `accent-neon`, `accent-2`, and the status colors as utilities.
- [X] T039 [P] Write `ui/src/theme/effects.css`:
  - `.neon-chrome`: glow and scanline overlay (`::after`, `pointer-events: none`).
  - `.grain`: a static inline-SVG `feTurbulence` background at `--grain-opacity`.
  - `.glitch`: RGB-split `text-shadow` and a `clip-path` slice keyframe, active only when `html[data-glitch="on"]`.
  - Everything is disabled under `html[data-effects="off"]`. Animations are disabled under `@media (prefers-reduced-motion: reduce)`. The light variant drops glow and scanlines.
  - Also add the circuit-trace SVG art component in `ui/src/theme/CircuitTrace.tsx`.
- [X] T040 [P] Load the fonts in `ui/src/theme/fonts.ts`:
  - `@fontsource/jetbrains-mono` 400/600, `@fontsource/chakra-petch` 600/700, and `@fontsource/yuji-boku` 400, bundled locally with no CDN.
  - Set `--font-mono` and `--font-sans` to JetBrains Mono, `--font-display` to Chakra Petch, and `--font-brand` to Yuji Boku.
  - Add the `BrandMark` component (尋 in `--font-brand` plus `FATHOM` in `--font-display`, with glow, and glitch on hover) in `ui/src/theme/BrandMark.tsx`.
- [X] T041 Write the contrast test first, in `ui/tests/unit/contrast.test.ts`:
  - Parse `tokens.css` for the three variants (dark-cyan, dark-magenta, light).
  - Assert WCAG ratios: at least 4.5:1 for foreground, muted, accent-neon, accent-2, and status colors on `--background` and `--card`, and for `--primary-foreground` on `--primary`; at least 3:1 for `--border`, `--chart-1..5`, and display text ≥ 18px.
  - Adjust token lightness (never hue) until it passes (SC-008).
- [X] T042 Implement `applyTheme(ui.theme)` in `ui/src/theme/apply.ts`. It toggles `.dark` and sets `data-accent`, `data-effects`, and `data-glitch` on `<html>`. Call it from `ui/src/main.tsx`, subscribed to `store.ui.theme`.
- [X] T043 [P] Re-skin the generated shadcn primitives in `ui/src/components/ui/*.tsx` to follow contracts/theme.md rules 4 and 5:
  - Remove rounded classes.
  - Selection uses `bg-primary text-primary-foreground` (inverse video).
  - Focus uses a 1px `ring` outline.
  - Buttons render as uppercase `--font-display` labels, with a neon border on hover/focus.
  - Dialogs and popovers use a 1px `--border` frame with a corner-notch accent in `--accent-neon`.

### Layout shell (contracts/layout.md)

- [X] T044 Write the width-class tests first, in `ui/tests/unit/width-class.test.ts`:
  - Boundaries 639→phone, 640→tablet, 1023/1024, 1439/1440, 1919/1920, 2559/2560.
  - `visibleSlots` is monotonic across the classes and the ≥3840 sub-step (FR-006).
- [X] T045 Implement `widthClassOf(px)`, `visibleSlots(class, px)`, and the `useWidthClass()` hook in `ui/src/layout/width-class.ts`:
  - `useWidthClass()` uses `matchMedia` listeners, not a resize polling loop.
  - It sets `data-width-class` on `<html>`, writes to `store.layout.widthClass`, and applies the density CSS variables (row height 44/28/24px, base font 15/13px).
- [X] T046 Implement the default `LayoutProfile` per class from the contracts/layout.md table in `ui/src/layout/defaults.ts`, and a `resolveProfile(class, saved?)` that truncates saved slots beyond `visibleSlots`.
- [X] T047 Implement `AppShell` in `ui/src/layout/AppShell.tsx`:
  - A top bar with the context and namespace pickers, a body, a status bar, and `<Toaster />`.
  - It chooses among `PhoneStack` (`ui/src/layout/PhoneStack.tsx`), `TabletTabs` (`ui/src/layout/TabletTabs.tsx`), and `SlotGrid` (`ui/src/layout/SlotGrid.tsx`, using react-resizable-panels) by width class.
  - Add the test hooks `data-slot="pane"`, `data-slot-index`, `data-pane-kind`, and `data-pane-id`.
- [X] T048 Implement `PaneHost` in `ui/src/layout/PaneHost.tsx`:
  - It renders the pane by `kind` (nav, list, inspector, tool) and keys its state off `store.panes[id]`, so moving between stack, tab, and slot never remounts the state (FR-013). An open dialog and in-progress input survive a width-class change (edge case).
  - It restores `scrollTop` on mount.
- [X] T049 [P] Implement `StatusBar` in `ui/src/components/StatusBar.tsx`: connection state, active context, the last backend notification, and a warning glyph when `conn.dropped > 0`. Apply the `.neon-chrome` class. The top bar in `ui/src/layout/AppShell.tsx` shows `BrandMark`.
- [X] T050 [P] Implement the shared `PaneState` UI components in `ui/src/components/PaneStates.tsx`: `Loading` (skeleton rows), `Empty`, `ErrorState` (message + retry), `Disconnected`, `NoLongerExists`, `NoKubeconfig` (first-run guidance), and `MetricsUnavailable` (FR-017). Empty and error states include the `CircuitTrace` art.

### CI (research R7/R9)

- [X] T051 Add a `ui` job to `.github/workflows/ci.yml`:
  - Set up pnpm and Node with caching, then run `pnpm -C ui install --frozen-lockfile`, lint, typecheck, test, and `pnpm -C ui exec playwright install --with-deps chromium webkit`, then e2e.
  - Upload `ui/playwright-report` when the job fails.
- [X] T052 Add a bindings drift step to `.github/workflows/ci.yml` and `pipeline/src/pipeline.rs`: `cargo test --test export_bindings`, then `git diff --exit-code ui/src/protocol/generated`.
- [X] T053 In `pipeline/src/pipeline.rs`:
  - Add the webkit2gtk-4.1 build packages (`libwebkit2gtk-4.1-dev libxdo-dev libayatana-appindicator3-dev librsvg2-dev`), Node, and pnpm to the Rust image.
  - Add `cargo clippy -p fathom-desktop -- -D warnings` and `cargo check --no-default-features --lib` to the `test` command.
  - Remove the `x86_64-pc-windows-gnu` target from `pipeline/src/targets.rs`.

### Integration hardening (found during implementation)

- [ ] T127 Do the first compile of `fathom-desktop`:
  - Run `cargo test -p fathom-desktop` (the pump/bridge tests from T024) and `cargo clippy -p fathom-desktop -- -D warnings`, and fix any compile errors in `ui/src-tauri/src/{lib,bridge,main}.rs`.
  - Then run `pnpm -C ui tauri dev` against a real kubeconfig, and confirm that the window opens, the status bar reaches `ready`, and closing the window shuts the backend down within 500 ms.
  - Confirm that a CI-style build with the placeholder `ui/dist/index.html` compiles.
- [ ] T128 Clear the pre-existing clippy baseline, so that `cargo clippy --all-targets -- -D warnings` passes (constitution Principle III). About 72 errors predate this feature, including `&Vec` params, `too_many_lines`, `cognitive_complexity`, and dead code.
  - Fix them mechanically without changing behavior, splitting long functions (for example `dispatch_cmd` in `src/backend/cmd.rs` and `src/app/mailbox.rs`) into helpers.
  - Egui-only code under `src/app/` that T125 will delete may instead get a scoped `#[allow]` with a `// removed by T125` comment.
  - `cargo test -p fathom` must stay green.

**Checkpoint**: In `pnpm -C ui dev:fake` the shell renders in the neon-noir theme at every width class, and in `tauri dev` the status bar shows `ready` from a real backend. T127 has run, the unit tests and the contrast test pass, and CI is green.

---

## Phase 3: User Story 1 — Browse and inspect cluster resources (Priority: P1) 🎯 MVP

**Goal**: Pick a context and namespace, browse every resource kind in live, searchable, sortable lists, and inspect any object with the same detail sections as today.

**Independent Test**:
1. With the fake transport (`two-contexts` + `pods-500` + `deployments`), switch context, search, and select.
2. Assert the rows, the emitted `Refresh*` commands, and the inspector sections.
3. Manually, run quickstart V1/V2 against a real cluster.

### Tests for User Story 1 ⚠️ (write first, must fail)

- [ ] T054 [P] [US1] Write the E2E test `ui/tests/e2e/us1-context-switch.spec.ts`. Selecting context `kind-b` sends the expected refresh commands, and late `kind-a` envelopes emitted afterwards never render, per FR-016 and acceptance scenario 1. It also covers these edge cases: with no kubeconfig or an unreachable cluster, the `NoKubeconfig` or `ErrorState` pane shows instead of a blank screen; while loading, `Loading` shows; after a subscription failure, `Disconnected` shows (FR-017). It ports `tests/gui_context_switching.rs`.
- [ ] T055 [P] [US1] Write the E2E test `ui/tests/e2e/us1-pods-list.spec.ts`:
  - 500 pods render.
  - Typing `web-` filters case-insensitively by name prefix and namespace, and clearing restores all rows.
  - Emitting an updated batch adds and removes a row without losing the search text or selection (scenarios 2–3).
  - It ports `tests/gui_pods.rs` and `tests/gui_search_filter.rs`.
- [ ] T056 [P] [US1] Write the E2E test `ui/tests/e2e/us1-inspector.spec.ts`. Selecting a pod sends the read commands, and the inspector shows the sections Metadata, Status/Conditions, Spec, Labels/Annotations, YAML, and Events (scenario 4). When a later list batch drops the selected object, the inspector shows `NoLongerExists` (edge case).
- [ ] T057 [P] [US1] Write the E2E test `ui/tests/e2e/us1-namespace-filter.spec.ts`. The global namespace filter carries from Pods into Deployments and ConfigMaps (scenario 5).
- [ ] T058 [P] [US1] Write the unit test `ui/tests/unit/search-sort.test.ts` for `matchesQuery` and the sort comparators (name, age, status, numeric columns).
- [ ] T059 [P] [US1] Write the unit test `ui/tests/unit/descriptors.test.ts`, covering every descriptor in `ui/src/resources/index.ts`:
  - it has priority-1 `name` and `status` columns;
  - the sum of its priority-1 `minWidth` is ≤ 358;
  - its `rowKey` is unique on its fixture;
  - `refresh(route)` returns a valid `BackendCmd`.

### Implementation for User Story 1

- [ ] T060 [US1] Define `ResourceDescriptor<R>`, `ColumnDef` (`priority: 1 | 2 | 3 | 4 | 5`, `minWidth`), `DetailSection`, and `ActionDescriptor` exactly as in data-model.md §5, in `ui/src/resources/types.ts`. Add the registry keyed by `SectionKey` in `ui/src/resources/index.ts`.
- [ ] T061 [US1] Implement `matchesQuery(row, q)` (case-insensitive name prefix or namespace) and the typed sort helpers in `ui/src/resources/search.ts`.
- [ ] T062 [US1] Implement `ResourceTable` in `ui/src/components/ResourceTable.tsx`:
  - TanStack Table + `useVirtualizer` (row height from a density CSS variable).
  - Header sort, a search input bound to `PaneState.search`, and multi-select checkboxes bound to `store.selection`.
  - Truncated cells use `title` and a long-press tooltip.
  - No horizontal scroll; the body uses `overflow-y: auto; overflow-x: hidden`.
  - Selecting a row calls `openInspector(ref)`.
- [ ] T063 [US1] Implement `Inspector` in `ui/src/components/Inspector.tsx` and `ui/src/components/DetailSections.tsx`:
  - Tabs for Details, YAML (a monospace `<pre>` that may scroll horizontally, per FR-010), and Events.
  - Sections rendered from `descriptor.detailSections`, which mirror `src/app/views/detail_sections.rs`.
  - It shows the columns hidden by the current column budget (FR-009).
- [ ] T064 [US1] Implement `ContextPicker` and `NamespacePicker` (shadcn Select, a searchable Command when there are more than 10 options) in `ui/src/components/ContextPicker.tsx` and `ui/src/components/NamespacePicker.tsx`. On change, they update `session`, reset the lists, and send the context/namespace commands from `src/protocol/v1/cmd.rs`.
- [ ] T065 [US1] Implement the `NavTree` in `ui/src/components/NavTree.tsx`:
  - Groups and sections mirror `src/sections/` and `src/app/ui_left.rs`.
  - Expanded groups come from `UiConfigLoaded.expanded_groups` and persist via debounced `SaveUiConfig`.
  - Selecting a section sets the route of the focused list pane and sends `descriptor.refresh(route)`.
- [ ] T066 [US1] Implement the pods descriptor in `ui/src/resources/pods.ts`:
  - Columns: name p1, status p1, ready p2, restarts p2, namespace p2, age p3, node p4, IP p5.
  - The pod detail sections.
  - `selectListEvent` for the pods `ResourceListEvent` arm.
  - Port the formatting from `src/app/views/pods.rs`.
- [ ] T067 [US1] Implement the deployments descriptor in `ui/src/resources/deployments.ts` and `ui/src/resources/workloads.ts`: columns for name, ready, up-to-date, available, and age, with the detail sections from `src/app/views/workloads.rs`.

**Checkpoint (MVP)**: The Pods and Deployments flows pass T054–T057 at the phone and wide projects.

- [ ] T068 [P] [US1] Workload descriptors in `ui/src/resources/{daemonsets,statefulsets,replicasets,replicationcontrollers,jobs,cronjobs}.ts`, ported from `src/app/views/workloads.rs`, `jobs.rs`, and `cronjobs.rs`.
- [ ] T069 [P] [US1] Cluster descriptors in `ui/src/resources/{nodes,namespaces,events,leases,priorityclasses,runtimeclasses}.ts`, ported from the matching `src/app/views/*.rs`.
- [ ] T070 [P] [US1] Network descriptors in `ui/src/resources/{services,endpoints,ingresses,ingressclasses,networkpolicies}.ts`, ported from the matching `src/app/views/*.rs`.
- [ ] T071 [P] [US1] Storage descriptors in `ui/src/resources/{persistentvolumes,persistentvolumeclaims,storageclasses}.ts`, ported from the matching `src/app/views/*.rs`, including the PVC disk usage.
- [ ] T072 [P] [US1] RBAC descriptors in `ui/src/resources/{roles,rolebindings,clusterroles,clusterrolebindings}.ts`, ported from the matching `src/app/views/*.rs`.
- [ ] T073 [P] [US1] Config descriptors in `ui/src/resources/{configmaps,secrets,resourcequotas,limitranges}.ts`. Secret values are masked by default, with a reveal toggle, matching `src/app/views/secrets.rs`.
- [ ] T074 [P] [US1] Policy and admission descriptors in `ui/src/resources/{horizontalpodautoscalers,poddisruptionbudgets,mutatingwebhooks,validatingwebhooks}.ts`, ported from the matching `src/app/views/*.rs`.
- [ ] T075 [US1] Register every descriptor in `ui/src/resources/index.ts`, and add one list fixture per kind to `ui/tests/fixtures/kinds/*.json`, so T059 covers all 34 kinds (FR-001; Flux is handled in US4).

**Checkpoint**: US1 is complete. All 34 core kinds browse and inspect in the new UI.

---

## Phase 4: User Story 2 — Adaptive density from phone to ultra-wide (Priority: P1)

**Goal**: Mobile first. Each wider class adds panes and columns, and on wide and ultra classes, users can pin, rearrange, and resize panes, with the arrangement remembered per class.

**Independent Test**: Run the Playwright projects `phone`, `laptop`, `wide`, and `ultra` against the fake transport, asserting the slot counts, column visibility order, state preserved across resizes, and no horizontal scroll (quickstart V3/V4).

### Tests for User Story 2 ⚠️

- [ ] T076 [P] [US2] Write the unit test `ui/tests/unit/column-budget.test.ts` for `computeVisibleColumns(columns, width)`:
  - Priority 1 is always shown.
  - Columns are added in ascending priority while the sum of `minWidth` fits.
  - A higher-priority column is never hidden while a lower one shows.
  - Columns are never below `minWidth`.
- [ ] T077 [P] [US2] Write the E2E test `ui/tests/e2e/us2-phone.spec.ts` (phone project):
  - List → select → the inspector is full-screen → Back returns to the list with its `scrollTop` preserved (scenario 1).
  - Touch targets are ≥ 44px.
  - `scrollWidth <= innerWidth`.
- [ ] T078 [P] [US2] Write the E2E test `ui/tests/e2e/us2-laptop.spec.ts` (laptop project). The list and inspector are side by side, and the nav is a collapsible rail (scenario 2).
- [ ] T079 [P] [US2] Write the E2E test `ui/tests/e2e/us2-ultra.spec.ts` (ultra project at 5120×1440):
  - At least 5 `[data-slot="pane"][data-slot-index]` elements are visible on load: nav, 2 lists, inspector, and the metrics/overview column, plus the dock (SC-002, scenario 3).
  - More columns show than at wide.
- [ ] T080 [P] [US2] Write the E2E test `ui/tests/e2e/us2-reflow.spec.ts`. Set the search text, selection, and scroll at wide, then resize with `page.setViewportSize` to phone and back; all three are preserved (scenario 5). Also resize while a dialog is open with half-typed input; the dialog stays open and the input is kept.
- [ ] T081 [P] [US2] Write the E2E test `ui/tests/e2e/us2-layout-persist.spec.ts`:
  - At wide, pin Deployments into list B and resize a slot. `SaveUiConfig` is sent with `layouts.wide`.
  - Reload with that config: the layout is restored at wide, and the default layout is used at laptop (scenario 6).

### Implementation for User Story 2

- [ ] T082 [US2] Implement `computeVisibleColumns` and the `useColumnBudget(paneRef, columns)` ResizeObserver hook in `ui/src/layout/column-budget.ts`, and wire it into `ResourceTable` (`ui/src/components/ResourceTable.tsx`) for column visibility.
- [ ] T083 [US2] Complete `PhoneStack` in `ui/src/layout/PhoneStack.tsx`:
  - A push/pop stack of `PaneId`s, with Back handled by an in-app button plus `popstate`.
  - A bottom tab bar (Resources, Logs, Forwards, More) with 44px targets.
  - Selecting a row pushes the inspector.
- [ ] T084 [US2] Complete `TabletTabs` in `ui/src/layout/TabletTabs.tsx`: a list/inspector split, with the nav in a shadcn `Sheet` drawer and overflowed panes as tabs.
- [ ] T085 [US2] Complete `SlotGrid` in `ui/src/layout/SlotGrid.tsx`:
  - Build `ResizablePanelGroup` slots from `resolveProfile`.
  - Laptop uses an icon nav rail with tooltips; desktop and up use the full `NavTree`.
  - Add a bottom `Dock` in `ui/src/layout/Dock.tsx` at desktop and up. At ultra it shows logs and events side by side.
- [ ] T086 [US2] Add pane pinning and rearranging:
  - A "Pin to slot" item in the pane header menu, and drag-to-swap between list slots (HTML5 DnD on the pane headers).
  - The `onLayout` sizes go into `store.layout.profile`.
  - Implement this in `ui/src/layout/pane-actions.ts` and `ui/src/layout/PaneHeader.tsx`.
- [ ] T087 [US2] Persist layout profiles in `ui/src/store/persist-ui.ts`:
  - Debounce by 500 ms, and only save on user-initiated changes (never on automatic resizes).
  - Send `SaveUiConfig` with `layouts[widthClass]`.
  - Hydrate from `UiConfigLoaded` through `resolveProfile`.
- [ ] T088 [US2] Add the ultra-only tool slot, showing an `OverviewSummary` placeholder (cluster counts from the list store) in `ui/src/views/OverviewSummary.tsx`. US4 replaces it with the full overview and metrics.

**Checkpoint**: US1 + US2 are complete. The P1 stories pass at phone, laptop, wide, and ultra in both Chromium and WebKit.

---

## Phase 5: User Story 3 — Act on resources safely (Priority: P2)

**Goal**: Delete, bulk delete, scale, and restart, each behind an explicit confirmation, with backend errors surfaced and no optimistic changes.

**Independent Test**: With the fake transport, trigger each action, assert the confirmation contents and that exactly one command is sent, and check that cancel sends nothing and that a failure shows a toast while the list stays unchanged.

### Tests for User Story 3 ⚠️

- [ ] T089 [P] [US3] Write the E2E test `ui/tests/e2e/us3-scale.spec.ts`. Scale a deployment to 3; the dialog shows current → new, and confirming sends `ScaleDeployment` exactly once (scenario 1).
- [ ] T090 [P] [US3] Write the E2E test `ui/tests/e2e/us3-bulk-delete.spec.ts`:
  - Select 3 pods, then Delete. The dialog lists every `namespace/name`, and Cancel sends nothing.
  - Confirming sends `BulkDelete`.
  - With more than 5 items, the user must type the count to confirm (scenario 2).
- [ ] T091 [P] [US3] Write the E2E test `ui/tests/e2e/us3-failure.spec.ts`. The fake emits an error result; an error toast shows the backend message, and the rows are unchanged (scenario 3).

### Implementation for User Story 3

- [ ] T092 [US3] Implement `ConfirmDialog` in `ui/src/components/ConfirmDialog.tsx`, using shadcn AlertDialog:
  - Props: `title`, `items: {namespace, name}[]`, `detail?: ReactNode`, `requireTypedCount` (true when there are more than 5 items), and `destructive`.
  - Esc and Cancel never send.
- [ ] T093 [US3] Implement `runAction(descriptor, action, refs)` in `ui/src/store/actions.ts`:
  - It registers `pending[request_id]`, sends the command, and resolves on the matching result event with a sonner toast (✓ or ✗ plus the backend message).
  - On success it clears the selection and refreshes the list. No optimistic removal.
- [ ] T094 [US3] Add `actions` to the descriptors in `ui/src/resources/*.ts`, using the delete/scale/restart commands in `src/protocol/v1/cmd.rs`:
  - `Delete*` for every kind that has one.
  - `Scale*` for deployments, statefulsets, replicasets, and replicationcontrollers.
  - `Restart*` for deployments, daemonsets, statefulsets, replicasets, and replicationcontrollers.
  - `BulkDelete` when more than one row is selected.
- [ ] T095 [US3] Implement `ScaleDialog` (a numeric input ≥ 0 showing current → new) in `ui/src/components/ScaleDialog.tsx`, and the row and pane-header action menus (DropdownMenu, plus a long-press context menu on touch) in `ui/src/components/ActionMenu.tsx`.

**Checkpoint**: US3 works on its own, on top of US1.

---

## Phase 6: User Story 4 — Logs, port forwards, metrics, and GitOps dashboards (Priority: P2)

**Goal**: Log streaming with follow, port-forward management with restore on start, the overview and metrics charts with an explicit "metrics unavailable" state, and the Flux dashboard.

**Independent Test**: Use the fake transport to stream log lines at high rate, emit port-forward and metrics events, and emit metrics-unavailable. Assert follow behavior, trimming, responsiveness, restored forwards, and the unavailable state.

### Tests for User Story 4 ⚠️

- [ ] T096 [P] [US4] Write the E2E test `ui/tests/e2e/us4-logs.spec.ts`:
  - `StartPodLogs` is sent. With follow on, the view is pinned to the bottom; scrolling up pauses follow, and End resumes it (scenario 1).
  - A burst of 20k lines at 2k/s keeps typing into search under 100 ms (measured with `performance.now`), and the DOM holds ≤ 10,000 lines (scenario 2).
  - Closing sends `StopPodLogs`.
- [ ] T097 [P] [US4] Write the E2E test `ui/tests/e2e/us4-metrics-unavailable.spec.ts`. With no Prometheus configured, the overview and metrics panes show `MetricsUnavailable`, not empty charts (scenario 3).
- [ ] T098 [P] [US4] Write the E2E test `ui/tests/e2e/us4-portforwards.spec.ts`:
  - Restored forwards from `RefreshPortForwards` show their live status (scenario 4).
  - Start, Stop, Edit, and Delete each open a confirmation. Cancel sends nothing, and Confirm sends the matching command exactly once.
  - It ports `tests/gui_portforwards.rs`.
- [ ] T099 [P] [US4] Write the E2E test `ui/tests/e2e/us4-flux.spec.ts`. The Flux dashboard renders the sources and kustomizations/helmreleases from the fixture, with their status glyphs.

### Implementation for User Story 4

- [ ] T100 [US4] Implement `LogViewer` in `ui/src/components/LogViewer.tsx`:
  - Virtualized lines from the `StreamState` ring buffer.
  - A follow toggle with auto-pause on scroll-up; End or a "follow" button resumes it.
  - Search highlighting, wrap and timestamp toggles, and horizontal scroll allowed inside the viewer only.
  - A "N lines truncated" marker.
  - Open and close send `StartPodLogs` / `StopPodLogs`.
- [ ] T101 [US4] Add a "Logs" action (for pods) that opens `LogViewer` in the dock (desktop and up), in the inspector tab (tablet/laptop), or as a pushed pane (phone), in `ui/src/resources/pods.ts` and `ui/src/layout/open-tool.ts`.
- [ ] T102 [P] [US4] Implement the `PortForwards` view in `ui/src/views/PortForwards.tsx`:
  - A table of local port → target and status, with actions Start, Stop, Edit (`UpdatePortForward`), and Delete, each behind `ConfirmDialog` (constitution Principle I, FR-002).
  - A "Port forward" action on pods and services.
  - Refresh on mount.
  - Ported from `src/app/views/portforwards.rs`.
- [ ] T103 [P] [US4] Implement `MetricsChart` (a Recharts line/area chart themed with `--chart-1..5`, a monospace legend, and no animation under reduced motion) in `ui/src/components/MetricsChart.tsx`.
- [ ] T104 [US4] Implement the `Overview` view in `ui/src/views/Overview.tsx`, ported from `src/app/views/overview.rs`:
  - Cluster summary cards, node metrics, and workload metrics charts.
  - `MetricsUnavailable` when the metrics events report no provider or an error.
  - This replaces `OverviewSummary` in the ultra tool slot (T088).
- [ ] T105 [US4] Add node and workload metrics panels to the inspector for nodes, pods, and deployments in `ui/src/components/DetailSections.tsx`, using the metrics store slice populated in `ui/src/store/handlers/list.ts`.
- [ ] T106 [P] [US4] Implement the `Flux` dashboard in `ui/src/views/Flux.tsx`, ported from `src/app/views/flux.rs`: sources, kustomizations, and helmreleases grouped with ready/suspended/failed glyphs, and an inspector for each Flux object. Add a Flux fixture in `ui/tests/fixtures/flux.json`.
- [ ] T107 [US4] Register the tools (`logs`, `portforwards`, `overview`, `metrics`, `flux`, `events`) as `ToolKind`s in `ui/src/layout/tools.ts`, so they can occupy dock and ultra slots and appear in the nav.

**Checkpoint**: US4 is complete. All the FR-003 tools are present.

---

## Phase 7: User Story 5 — Keyboard-driven operation and command palette (Priority: P3)

**Goal**: Operate every section, context switch, and common action from the keyboard, with a Ctrl/Cmd+K palette.

**Independent Test**: With keyboard only in Playwright, open the palette, navigate to Services, move and select rows with the arrow keys and Enter, and run an action.

### Tests for User Story 5 ⚠️

- [ ] T108 [P] [US5] Write the E2E test `ui/tests/e2e/us5-palette.spec.ts`. Ctrl/Cmd+K, type "svc", then Enter navigates to Services. Context switch and "Delete selected" are listed (scenario 1).
- [ ] T109 [P] [US5] Write the E2E test `ui/tests/e2e/us5-keyboard.spec.ts`:
  - A focused list: ArrowDown then Enter selects the next row and opens the inspector (scenario 2).
  - Tab order reaches every pane, and the focus ring is always visible.
  - Run an axe-core scan with `@axe-core/playwright` and assert no serious violations.

### Implementation for User Story 5

- [ ] T110 [US5] Implement the shortcut registry in `ui/src/app/shortcuts.ts`, mirroring the current egui shortcuts in `src/app/` (grep for `consume_key` / `key_pressed`):
  - `/` focuses search, `j/k` and the arrows move, `Enter` opens, `Esc` closes or goes back, `Del` deletes, `g p` goes to Pods, etc.
  - Scoped per pane focus.
  - A `?` overlay lists every binding.
- [ ] T111 [US5] Implement `CommandPalette` in `ui/src/components/CommandPalette.tsx`, using shadcn Command/cmdk. It covers every section (from the descriptor registry plus the tools), context and namespace switching, theme toggles, and the actions valid for the current selection.
- [ ] T112 [US5] Add keyboard row navigation to `ResourceTable` in `ui/src/components/ResourceTable.tsx`:
  - A roving `aria-activedescendant` on a `role="grid"`, with the block cursor `▌` on the focused row (theme rule 4).
  - `Space` toggles selection.
  - The virtualizer scrolls the focused row into view.

**Checkpoint**: US5 is complete.

---

## Phase 8: User Story 6 — Settings and integrations (Priority: P3)

**Goal**: Integrations, MCP, and appearance settings, with existing on-disk state read unchanged and the MCP server independent of the screen.

**Independent Test**: Seed the fake `UiConfigLoaded` and integrations events from an egui-era `~/.fathom/` snapshot, verify every value renders, and check that toggles send the right commands.

### Tests for User Story 6 ⚠️

- [ ] T113 [P] [US6] Write the E2E test `ui/tests/e2e/us6-settings.spec.ts`:
  - Using a fixture copied from a real egui-era config, the integrations and MCP auto-start values render (scenario 1).
  - Toggling MCP auto-start sends `SetMcpAutoStart`.
  - The theme toggles (magenta accent, light, effects off, glitch on) update the `<html>` attributes immediately and send `SaveUiConfig`.
  - With `page.emulateMedia({ reducedMotion: 'reduce' })`, no element has a running animation (checked with `document.getAnimations()`), even with glitch on.
  - It ports `tests/gui_integrations.rs`.
- [ ] T114 [P] [US6] Add a Rust integration test in `tests/upgrade_compat.rs` (SC-009). Copy an egui-era `~/.fathom/` fixture (ports.json, metrics, integrations, credentials, config.json) into a temp HOME, then run the load paths: ports load, integrations load, and `LoadUiConfig` returns `ui: None` plus the existing `expanded_groups` and `mcp_auto_start`.

### Implementation for User Story 6

- [ ] T115 [P] [US6] Implement the `Integrations` view in `ui/src/views/Integrations.tsx`, ported from `src/app/views/integrations.rs`: the Prometheus/metrics provider config and credentials, using the existing integration commands. Credential fields are write-only, and the UI never displays saved secrets.
- [ ] T116 [P] [US6] Implement the `Mcp` view in `ui/src/views/Mcp.tsx`, ported from `src/app/views/mcp_integration.rs`: server status, endpoint `127.0.0.1:18288`, start/stop, the auto-start toggle (`SetMcpAutoStart`), and copyable client config snippets.
- [ ] T117 [US6] Implement the `Settings` view in `ui/src/views/Settings.tsx`:
  - Appearance: mode dark/light, accent cyan/magenta, effects on/off, glitch (default off), and a density override.
  - Keyboard shortcuts reference.
  - About/version.
  - Changes are applied via `applyTheme` and persisted via `SaveUiConfig`.
- [ ] T118 [US6] Verify the MCP server runs regardless of the UI:
  - The MCP server is started by the backend auto-start path, not by a view mount.
  - Add a `tauri-driver` smoke test in `ui/tests/driver/smoke.test.ts` (Linux only). Launch the built app, wait for the status bar to show `ready`, `curl` `127.0.0.1:18288` while the Pods view is open, then close the window and assert a clean exit.

**Checkpoint**: All six user stories are functional in the new UI.

---

## Phase 9: Polish, Cutover & Cross-Cutting Concerns

**Purpose**: Performance budgets, release packaging, parity sign-off, and removing egui (FR-024, FR-025, SC-001, SC-003–SC-006).

- [ ] T119 [P] Add a performance E2E test in `ui/tests/e2e/perf-10k.spec.ts` (Chromium and WebKit, wide project):
  - Load `pods-10k.json`, then emit updates every 250 ms.
  - Assert that search keystroke-to-render is under 100 ms and that no long task exceeds 100 ms, using `PerformanceObserver` (SC-004).
- [ ] T120 [P] Idle CPU check (SC-005):
  - Add `ui/tests/driver/idle.test.ts` (Linux, `tauri-driver`). Open Pods, idle for 60 s, and sample the process CPU from `/proc/<pid>/stat`. Assert it stays under 2% of one core.
  - Document the manual procedure in `specs/001-tauri-shadcn-frontend/quickstart.md` under V13.
- [ ] T121 [P] Startup baseline (SC-003): record egui launch-to-populated-pods time, then compare Tauri, in a script `scripts/measure-startup.sh`. Record the results in `specs/001-tauri-shadcn-frontend/research.md` under a new "Baselines" heading.
- [ ] T122 Replace the release build matrix in `.github/workflows/release.yml` with `tauri-apps/tauri-action@v0`:
  - `ubuntu-22.04` x86_64 and `ubuntu-22.04-arm` aarch64 (deb, rpm, AppImage)
  - `macos-latest` with `--target aarch64-apple-darwin` and `--target x86_64-apple-darwin` (dmg)
  - `windows-latest` (msi, nsis)
  - Unify the archive names as `fathom-<version>-<os>-<arch>` (FR-024).
- [ ] T123 [P] Update `README.md` with the new prerequisites (webkit2gtk-4.1, Node, pnpm), the dev, test, and build commands from quickstart.md, and the screenshots at phone and ultra widths.
- [ ] T124 Parity sign-off (SC-001). Create `specs/001-tauri-shadcn-frontend/checklists/parity.md`, listing every item from FR-001–FR-003 and each `src/app/views/*.rs` file with the new-UI equivalent and the E2E test that covers it. Every row must be checked before T125.
- [ ] T125 Remove the egui UI (FR-025). Delete:
  - `src/app/`, `src/main.rs`, `tests/gui_*.rs`, and `tests/common/` if it is GUI-only.
  - The egui deps and the `egui-ui` feature from `Cargo.toml`, and the `[[bin]]` for `fathom` (the shipping binary is now `fathom-desktop`, bundled as "Fathom").
  - `#[cfg(feature = "egui-ui")]` in `src/lib.rs`.
  - Also update the `pipeline/src/pipeline.rs` and `.github/workflows/ci.yml` steps that referenced the egui binary.
- [ ] T126 Run the full quickstart.md validation (V1–V13) on each Tier 1 platform (Ubuntu 26.04 LTS x86_64 and arm64, Windows 11 x86_64, macOS 26 Tahoe arm64 and x86_64), using the T122 artifacts. Record the OS build, the artifact, and the pass/fail per scenario in `specs/001-tauri-shadcn-frontend/checklists/release-validation.md`. Where GitHub has no hosted runner for a Tier 1 OS, the manual run recorded there is required before publishing (constitution, Supported Platforms).

---

## Dependencies & Execution Order

### Phase dependencies

```text
Phase 1 Setup ──▶ Phase 2 Foundational ──▶ US1 (P1) ──▶ US2 (P1)
                                             │           │
                                             ├──▶ US3 (P2)  (needs descriptors + ResourceTable)
                                             ├──▶ US4 (P2)  (needs store streams; dock needs US2 SlotGrid)
                                             ├──▶ US5 (P3)  (needs ResourceTable + descriptor registry)
                                             └──▶ US6 (P3)  (needs only Foundational + AppShell)
all stories ──▶ Phase 9 (T124 parity ──▶ T125 delete egui ──▶ T126 validation)
```

- **T127** (first desktop compile) needs the webkit2gtk-4.1 dev libs and gates the Phase 2 checkpoint. **T128** (the clippy baseline) gates a green CI, but it blocks no story work.
- **US1** is the MVP and builds the shared `ResourceTable`, `Inspector`, and descriptor registry.
- **US2** reworks the layout around the US1 components. Its column budget (T082) touches `ResourceTable.tsx`, so it runs after T062.
- **US3, US4, US5, and US6** can proceed in parallel once US1 T060–T067 are done. US4's dock placement (T101) needs US2's T085.
- **US6** is the most independent. It can start right after Phase 2.

### Within each story

- Write the E2E and unit tests first and confirm they fail, then implement.
- Descriptors (T068–T074) are independent files and fully parallel.
- The Rust protocol derive tasks T012 → T013 → T014 run in order, because they depend on each other's types. T019 and T020 then run in parallel.

### Parallel opportunities

- Phase 1: T006–T010 run together once T003–T005 are done.
- Phase 2:
  - The transports T029/T030/T031/T032 run in parallel after T028.
  - The theme tasks T039/T040/T043 run in parallel.
  - The status and pane-state components T049/T050 run in parallel.
  - The Rust track (T011–T027) and the frontend track (T028–T050) run in parallel until T037, which needs the generated bindings from T021.
- US1: all tests T054–T059 together, and descriptors T068–T074 together (7 parallel streams).
- US2: tests T076–T081 together.
- US4: T102, T103, and T106 in parallel with T100.
- US6: T115 and T116 in parallel.

## Parallel examples

```text
# US1 descriptor fan-out (after T060–T067):
T068 workloads | T069 cluster | T070 network | T071 storage | T072 rbac | T073 config | T074 policy/admission

# US2 tests:
T076 column-budget.test.ts | T077 us2-phone | T078 us2-laptop | T079 us2-ultra | T080 us2-reflow | T081 us2-layout-persist

# Cross-story, after US1 core (T067):
Dev A: US3 (T089–T095)   Dev B: US4 (T096–T107)   Dev C: US6 (T113–T118)
```

## Implementation strategy

1. **MVP (weeks 1–2)**:
   - Phases 1–2, then US1 through T067 (Pods + Deployments).
   - Stop and validate with quickstart V1/V2 in `tauri dev`. This proves the bridge, the store, the theme, and the first descriptors.
2. **P1 complete**: T068–T075 (all kinds) plus US2. The new UI is usable daily for read-only work at any width, and egui still ships.
3. **P2**: US3 (actions) and US4 (logs, forwards, metrics, Flux) in parallel. After this the new UI can replace egui for most users.
4. **P3**: US5 and US6.
5. **Cutover**: T119–T126. Switch the release to Tauri, sign off parity, then delete egui in one reviewable PR (T125).

**Totals**: 128 tasks. T127 and T128 were added to Phase 2 after implementation began, and are numbered out of order so the existing IDs stay stable.
- Setup: 10
- Foundational: 45
- US1: 22
- US2: 13
- US3: 7
- US4: 12
- US5: 5
- US6: 6
- Polish/cutover: 8
