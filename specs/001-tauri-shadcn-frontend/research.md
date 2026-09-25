# Phase 0 Research: Tauri + React Frontend

**Feature**: [spec.md](./spec.md) | **Plan**: [plan.md](./plan.md) | **Date**: 2026-09-21

Every Technical Context unknown is resolved below. Sources are the Tauri v2 docs (v2.tauri.app), shadcn/ui docs (ui.shadcn.com), and a code inventory of this repository taken on the date above.

---

## R1. How the webview talks to the existing backend

**Decision**: Keep the existing `BackendCmd` / `BackendEventEnvelope` protocol and the backend thread unchanged. Bridge it to the webview with:

- one Tauri command, `send_cmd(cmd: BackendCmd)`, which forwards to the existing crossbeam sender, and
- one `tauri::ipc::Channel<Vec<BackendEventEnvelope>>` opened by `subscribe_events`. A bridge thread blocks on the crossbeam receiver, **coalesces envelopes into batches (flush every ≤16 ms or 256 envelopes)**, and sends them down the channel.

Tauri *events* (`emit`) are used only for rare global broadcasts that must reach every window, and at first nothing needs them.

**Rationale**:
- The protocol has 137 command variants and 90 event variants, and the backend has no UI-toolkit code. The seam already exists, so the webview just becomes a new client of it.
- The Tauri docs say events are "not designed for low latency or high throughput" and recommend Channels for ordered streaming.
- Batching is necessary because list refreshes and log chunks arrive in bursts. One IPC message per envelope would flood the webview.
- A blocking `recv()` on the bridge thread fixes the continuous-repaint problem at its root. Nothing happens unless the backend sends something (SC-005).

**Alternatives considered**:
- *One `#[tauri::command]` per operation, with per-subscription Channels for watches.* This is the idiomatic Tauri shape, but it means rewriting how ~137 operations are dispatched and changing the backend's broadcast model. It's a lot of risk for no user-visible gain. We can revisit this per operation later, e.g. for logs, if profiling shows a need.
- *Tauri events for everything.* Rejected because of the docs' throughput warning.
- *Local HTTP/WebSocket server, which the MCP axum server already provides.* It works in a plain browser too, but adds a port, auth concerns, and a second transport. It's kept as a future option for a web build.

## R2. Rust → TypeScript contract

**Decision**: Use **ts-rs** (stable, v12) on the protocol types. Every type reachable from `BackendCmdV1` and `BackendEventEnvelopeV1` gets `#[derive(Serialize, Deserialize, TS)]` with an explicit, stable serde representation: adjacently tagged enums, `#[serde(tag = "type", content = "data")]`. A `cargo test` target exports them to `ui/src/protocol/generated/`. CI fails if the generated files differ from what's committed (FR-026).

**Rationale**:
- The bridge exposes two commands. Most of what tauri-specta offers is typed wrappers per command, so we'd get little from it.
- ts-rs is stable and respects serde attributes.
- tauri-specta is still `2.0.0-rc.25` with an exact `specta` pin.

**Alternatives considered**:
- tauri-specta (an RC, and it pins versions).
- Hand-written TS types, which drift silently.
- JSON Schema via schemars plus a codegen step, which has more moving parts.

**Required backend change**: add serde derives to the types that lack them today: `BackendCmdV1`, everything in `protocol/v1/event.rs`, `ids.rs` (`RequestId`, `StreamId`, `Correlation`), `route.rs` (`SectionKey`, `ScopeKind`, `Route`), `cronjobs.rs`, and `jobs.rs`. Most payload structs already derive serde. `k8s-openapi` types must not leak into the contract, and today they don't; the payloads are Fathom's own summary and detail DTOs.

## R3. Crate and workspace layout

**Decision**:
- **Frontend**: `ui/`, a pnpm + Vite + React + TypeScript app.
- **Tauri crate**: `ui/src-tauri/`, a new workspace member named `fathom-desktop`.
  - It depends on the root `fathom` crate as a **library** with `default-features = false`.
  - It uses the mobile-ready shape: `[lib] crate-type = ["staticlib","cdylib","rlib"]`, plus a `run()` marked `#[cfg_attr(mobile, tauri::mobile_entry_point)]` and a thin `main.rs`.
- **Root crate**: gains a cargo feature `egui-ui` (on by default). It gates `app`, `main.rs`'s GUI path, and the egui, eframe, egui_plot, rfd, image, and egui_kittest dependencies. The old UI keeps building until parity (FR-025), and the Tauri crate never links egui.
- **Headless mode**: `fathom-desktop --only-mcp` is handled before the Tauri builder starts, which keeps FR-005.

**Rationale**: This is the standard Tauri layout and keeps the old and new UIs coexisting. `lib.rs` already exports `backend`, `protocol`, `mcp_server`, and `sections`.

**Cleanups the move requires**:
- Views call `crate::backend::persistence` directly (navigation, mcp_integration). Those calls move behind commands (`LoadUiConfig`, `SaveUiConfig`, `SetMcpAutoStart`) so the UI talks only through the protocol.
- `main.rs` re-declares modules instead of using the lib. That gets fixed as part of the feature gate.

## R4. Frontend stack

**Decision** (tools fixed by the user; libraries chosen here):

| Concern | Choice |
|---|---|
| Package manager | pnpm (workspace root at `ui/`) |
| Build / dev server | Vite, with the Tauri-recommended config (`strictPort`, `TAURI_DEV_HOST`, `envPrefix`, `watch.ignored: src-tauri`) |
| UI framework | React 19 + TypeScript (strict) |
| Components | shadcn/ui (Radix base), Tailwind CSS v4 via `@tailwindcss/vite` |
| Tables | TanStack Table (the basis of shadcn's data-table) + **TanStack Virtual** for row virtualization, which shadcn does not provide |
| Layout panes | shadcn `resizable` (react-resizable-panels) |
| Command palette | shadcn `command` (cmdk) |
| Charts | shadcn `chart` (Recharts) |
| Toasts | shadcn `sonner` |
| Client state | Zustand: one normalized store fed by the event reducer (R6) |
| Navigation state | A small typed router over `history` (hash-based, so it works in the Tauri custom protocol and in browser-mode tests). No router library until a real need shows up. |
| Unit / component tests | Vitest + Testing Library (jsdom) |
| E2E tests | Playwright (R7) |
| Lint / format | ESLint (typescript-eslint) + Prettier |

## R5. Responsive density model

**Decision**:
- **Six width classes** are computed from the app viewport's logical width:

  | Class | Width |
  |---|---|
  | phone | < 640 |
  | tablet | 640–1023 |
  | laptop | 1024–1439 |
  | desktop | 1440–1919 |
  | wide | 1920–2559 |
  | ultra | ≥ 2560 |

- **Layout** uses a slot-based pane layout (see [contracts/layout.md](./contracts/layout.md)). Each class declares how many slots are visible and which default panes fill them. The pane count only goes up as width goes up (FR-006).
- **Tables** give every column a `priority` (1 = always shown). A column is visible when its priority is at or below the pane's column budget. The budget comes from the **pane's own width**, measured with a container query or ResizeObserver, not from the viewport, so a narrow pane on an ultrawide still hides columns correctly (FR-009).
- **Row density** also scales: 28 px rows by default, 24 px from `wide` up, and 44 px touch targets on `phone` and `tablet` (FR-012).

**Rationale**:
- Mobile-first CSS with container queries matches "the wider, the denser", and pane-level budgets compose correctly when several lists are open at once.
- The Tauri webview baselines (Safari 13 target on macOS/Linux WebKitGTK) support ResizeObserver. For container queries, a ResizeObserver fallback is used where needed.

**Alternatives considered**:
- Pure viewport media queries: these break when two lists share an ultrawide.
- A free-form tiling window manager: too much for v1. Pinning and resizing within slots covers FR-011.

## R6. Event handling in the UI

**Decision**:
- A single `ingest(batch)` reducer applies envelopes to a normalized store: lists keyed by `(context, route, kind)`, details keyed by `(context, kind, ns, name)`, and streams keyed by `StreamId`.
- It drops envelopes whose `correlation` or context is stale (FR-016).
- It updates by structural diff, so unchanged rows keep their identity and React skips them (FR-013).
- Log streams go into a ring buffer (default 10,000 lines per stream) and render through a virtualized list (FR-014).

**Rationale**: This matches today's mailbox semantics (list events fan out, others go to the active view) without the `*mut dyn ViewComponent` casts, which are a symptom of fighting immediate mode.

## R7. Testing strategy

**Decision**:
- **Vitest** covers pure logic: the reducer, search and sort, column budgets, width-class selection, and the command builders.
- **Playwright** runs the Vite build in Chromium and WebKit against a **fake backend transport**. The transport is chosen at startup (`transport = tauri | fake`). The fake is a scripted in-page backend that plays fixture envelopes and records commands, which mirrors today's `egui_kittest` harness (injects events, asserts emitted commands).
  - Every P1 story runs at 390×844 (phone) and 2560×1440 (wide). Ultrawide checks use 5120×1440 (FR-023).
  - WebKit is included because Linux and macOS ship WebKit-based webviews.
- A thin **smoke test of the real Tauri binary** on Linux CI uses `tauri-driver` (WebDriver) to launch, confirm the bridge is connected, and quit.
- Contract drift is guarded by the ts-rs export check (R2) plus Rust unit tests that round-trip every command and event variant through serde_json.

**Alternatives considered**:
- Playwright-only against the real binary: Tauri doesn't support that on all OSes, and it's slower and flakier.
- Keeping `egui_kittest`: that tool is being deleted.

## R8. Retro theme (neon-noir terminal)

This section was revised on 2026-09-21 after a branding review, and it supersedes the earlier phosphor-CRT decision.

**Decision**:
- **Tokens.** Tailwind v4 CSS variables in OKLCH, using shadcn's token names plus the Fathom extensions `--accent-neon`, `--accent-2`, `--glow`, `--scanline-opacity`, and `--grain-opacity`, and `data-accent="cyan"|"magenta"`. The full table is in [contracts/theme.md](./contracts/theme.md).
- **Radius**: `--radius: 0`.
- **Dark (default)**: an abyssal navy background `#070b1a` with ice text `#d6f6ff`, neon cyan `#3ef2ff` or magenta `#ff2bd6` as the accent, and violet `#8b6cff` for structure. Hot pink and purple are never used for body text, because they fail AA on navy.
- **Light**: a "blueprint" variant, with navy ink on pale blue-white paper.
- **Fonts**: self-hosted via `@fontsource`, since the app must work offline.
  - **JetBrains Mono** for all data and body text.
  - **Chakra Petch** (angular, blocky) for display and chrome.
  - **Yuji Boku** (brush) only for the 尋 mark. Fontsource's unicode-range subsets keep this to one small chunk.
- **Brand**: 尋 (*hiro*) is literally the Japanese unit "fathom", so the mark is a pun, not decoration for its own sake.
- **Effects**:
  - Glow, scanlines, and static grain appear on chrome.
  - Glitch (RGB split) plays on brand hover, context switch, and error banners, and is opt-in.
  - Circuit-trace SVGs appear in empty and error states.
  - Effects never apply to rows or log lines. They are disabled by reduced motion and by a settings toggle (FR-019).
- **Selection and focus** use inverse video with a block-cursor marker.
- **Contrast**: an automated check fails CI below AA (SC-008).

**Rationale**:
- "Fathom" means depth. Abyssal navy with bioluminescent neon ties the name to the look.
- Keeping the terminal structure keeps the dense data readable.
- The cyberpunk energy lives in color, type, and chrome, where it costs neither legibility nor idle CPU.

**Alternatives considered**:
- Pure phosphor CRT (the original choice): coherent, but monochrome, and less distinctive.
- Full cyberpunk, with glitch and texture everywhere: it hurts dense-table legibility and the idle-CPU budget (SC-005).

## R9. Build, packaging, CI

**Decision**:
- **Release**: replace the cargo-only release job with `tauri-apps/tauri-action` on a matrix:

  | Runner | Target | Bundles |
  |---|---|---|
  | `ubuntu-22.04` | x86_64 | deb, rpm, AppImage |
  | **`ubuntu-22.04-arm`** | aarch64 | deb, rpm, AppImage; native, because AppImage can't be cross-compiled and cross-rs is no longer enough |
  | `macos-latest` | x86_64 + aarch64 | dmg |
  | `windows-latest` | x86_64 | msi + nsis, WebView2 download bootstrapper |

  The release archive names are unified to fix the `arm64`/`aarch64` mismatch the graph flagged.
- **Dagger pipeline**: the base image gains `libwebkit2gtk-4.1-dev libxdo-dev libayatana-appindicator3-dev librsvg2-dev`, Node LTS, and pnpm. `test` adds `pnpm -C ui lint typecheck test` and the Playwright suite. The Windows-gnu cross target is dropped from the pipeline because Tauri's MSI needs a Windows host; releases keep the msvc build.
- **Icons**: `pnpm tauri icon assets/icon.png` generates every platform icon, replacing `build.rs`'s ico conversion for the desktop crate.

## R10. Mobile readiness (not in this feature's scope)

**Decision**: Only the crate shape (R3) and the layout (R5) are made mobile-ready. Not in scope: `tauri android init` / `ios init`, the question of where the backend runs on mobile, and credential handling.

**Known future blockers**:
- There's no `~/.kube/config` on a phone, so an import flow is needed.
- exec credential plugins (aws, gke, kubelogin) can't be spawned on iOS or Android.
- Local port-forwarding has limited value on mobile.
- Desktop-only plugins (global-shortcut, window-state, updater, tray) must be gated by `platforms` in capabilities.

## Baselines

Measured on the author's Linux dev box (Ubuntu, Wayland session, WebKitGTK 2.52.6) with the release build `target/release/fathom-desktop`, three kubeconfig contexts, none of them reachable through the app at measurement time. Only what was actually measured is recorded.

| Metric | Method | Result |
|---|---|---|
| Tauri: launch to backend context-list handoff (5 runs) | `scripts/measure-startup.sh --runs 5` (spawn to the `Sending N contexts to UI` log line) | 920, 946, 922, 918, 931 ms; median 922 ms |
| Tauri: idle CPU, launch view, 60 s after a 10 s settle | `scripts/measure-idle-cpu.sh` (utime+stime of the app and its WebKit child processes, % of one core) | 0.95% (budget < 2%) |
| SC-003: launch to populated Pods (budget < 3 s; egui baseline obsolete) | `ui/tests/driver/smoke.test.ts` (tauri-driver, kind cluster) and `scripts/measure-startup.sh --runs 3` | Pods populated in ~1.6 s (single run, includes driver polling); context handoff median 1036 ms, PASS |
| SC-005: idle on the Pods view, 60 s after 10 s settle | `ui/tests/driver/idle.test.ts` (tauri-driver, kind cluster) | 0.10% of one core (budget < 2%) |
| SC-004 (10,000 pods, 250 ms updates): keystroke under 100 ms, no long task over 100 ms | `ui/tests/e2e/perf-10k.spec.ts`, `chromium-wide` | passes (budgets asserted, per-run values not recorded); `webkit-wide` not run, WebKit here lacks a system library (`libevent-2.1-7t64`) |

The egui comparison is obsolete (egui removed, T125); SC-003 is checked against its absolute 3 s budget.
