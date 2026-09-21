# Implementation Plan: Replace Desktop UI with a Responsive, Retro-Themed Web-Technology Frontend

**Branch**: `001-tauri-shadcn-frontend` | **Date**: 2026-09-21 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `specs/001-tauri-shadcn-frontend/spec.md`

## Summary

Replace the egui/eframe UI (`src/app/`, ~13.8k LOC, plus ~1.2k LOC of `egui_kittest` tests) with a Tauri v2 desktop shell hosting a React + TypeScript + shadcn/ui frontend styled as a neon-noir terminal.

The kube-rs/tokio backend (~13.5k LOC), the command/event protocol (~2.4k LOC), the MCP server, and the `~/.fathom/` settings are kept. The webview joins the existing protocol as a new client. It sends commands through one Tauri command, and backend events arrive in batches over one `ipc::Channel`. The TypeScript protocol types are generated from Rust with ts-rs.

The layout is mobile-first across six width classes. Visible panes only increase with width, and table columns are budgeted per pane, so an ultrawide shows most of the app at once. The old UI stays behind an `egui-ui` cargo feature until parity, then is deleted.

## Technical Context

**Language/Version**: Rust 2024 edition (existing workspace; rustc ≥ 1.85); TypeScript 5.x (strict); Node LTS

**Primary Dependencies**:
- **Rust**: tauri 2.x, ts-rs 12, plus the existing tokio, kube 2, rmcp, axum, and crossbeam-channel.
- **Frontend**: React 19, Vite, Tailwind CSS v4, shadcn/ui (Radix), TanStack Table + Virtual, react-resizable-panels, cmdk, Recharts, sonner, Zustand, `@tauri-apps/api`, and `@fontsource` (JetBrains Mono, Chakra Petch, Yuji Boku).

**Storage**: The existing JSON files under `~/.fathom/` (ports, metrics, integrations, credentials, config). Layout profiles go in a new `ui` section of `config.json`, reached through backend commands. No format changes to existing keys (FR-004, SC-009).

**Testing**:
- Rust: `cargo test`, including serde round-trips for every protocol variant and the ts-rs export check.
- Frontend: Vitest + Testing Library for units, and Playwright (Chromium + WebKit) against a fake-backend transport.
- One `tauri-driver` smoke test on Linux.

**Target Platform**:
- Tier 1 (constitution): Ubuntu 26.04 LTS x86_64/arm64 (WebKitGTK 4.1), Windows 11 x86_64 (WebView2), and macOS 26 Tahoe arm64/x86_64 (WKWebView).
- Tier 2 (best effort): older Ubuntu, Windows 10, and macOS 14–15.
- Mobile is layout-ready only (out of scope).

**Project Type**: Desktop app (Tauri) with a Rust core library and a web frontend.

**Performance Goals**:
- Cold start to a populated pods list in under 3 s, and no slower than the egui baseline (SC-003).
- Input response under 100 ms with a live 10k-row list (SC-004).
- Logs at 1,000+ lines/s without jank.
- Idle CPU under 2% of one core (SC-005).

**Constraints**:
- No horizontal page scroll at any width ≥ 360 px.
- WCAG 2.2 AA contrast.
- `prefers-reduced-motion` must be honored.
- Works offline (no CDN fonts or assets).
- The Vite build target is the Tauri default (`safari13` / `chrome105`).

**Scale/Scope**:
- 35 resource kinds × (list + inspector), plus 6 tools/dashboards and the shell.
- 137 command variants and 90 event variants.
- Clusters up to about 10k pods.

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

Evaluated against constitution v1.0.0 (ratified 2026-09-21):

| Check | Status |
|---|---|
| I. Operator-first, explicit mutation | ✅ Every action, including port-forward start/stop/edit, goes through `ConfirmDialog`. No optimistic updates. Backend errors are shown verbatim. |
| II. One contract, backend owns the cluster | ✅ Two Tauri commands bridge the existing protocol. The TS types are generated with ts-rs. Direct persistence calls move behind commands. |
| III. Tested at the boundaries | ✅ Fake-transport E2E tests per story, serde round-trips, a bindings drift check, and CI gates (fmt, clippy, lint, typecheck, Vitest, Playwright). |
| IV. Density scales with width | ✅ See contracts/layout.md: monotonic slots, column budgets, no horizontal scroll, AA contrast test, effects toggle, and reduced motion honored. |
| V. Fast and quiet | ✅ Blocking-recv bridge, 16 ms batches, virtualization, ring buffers, a 10k perf test, an idle CPU test, and a startup baseline. |
| VI. User data is sacred | ✅ `ui.*` is additive. Invalid config is moved to `ui_invalid`, never deleted. Secrets are masked. The upgrade-compat test covers this. |
| Platforms Tier 1 | ✅ Ubuntu 26.04, Windows 11, and macOS 26 are validated in T126. Builds may come from older images. |

**Post-design re-check**: No new violations. Adding one new workspace crate (`fathom-desktop`) and one new JS package (`ui/`) is the minimum Tauri needs.


## Project Structure

### Documentation (this feature)

```text
specs/001-tauri-shadcn-frontend/
├── plan.md              # This file
├── research.md          # Phase 0: decisions R1–R10
├── data-model.md        # Phase 1: UI-side entities and state
├── quickstart.md        # Phase 1: how to run and validate
├── contracts/
│   ├── ipc.md           # Rust ⇄ webview bridge contract
│   ├── layout.md        # Width classes, slots, and column budgets
│   └── theme.md         # Retro token contract
├── checklists/requirements.md
└── tasks.md             # Phase 2 (/speckit-tasks; not created here)
```

### Source Code (repository root)

```text
Cargo.toml                      # workspace members: ".", "pipeline", "ui/src-tauri"
                                # root crate gains [features] default = ["egui-ui"]
src/
├── lib.rs                      # `app` module gated by cfg(feature = "egui-ui")
├── main.rs                     # egui binary (required-features = ["egui-ui"]); removed at parity
├── backend.rs, backend/        # UNCHANGED except: persistence reached only via commands;
│                               #   new LoadUiConfig/SaveUiConfig handling
├── protocol/v1/                # + Serialize/Deserialize/TS derives, adjacently tagged enums
├── mcp_server.rs               # unchanged
├── sections/                   # unchanged (serde already)
└── app/                        # egui UI: frozen, then deleted at parity (FR-025)

ui/                             # NEW: pnpm + Vite + React + TS frontend
├── package.json, pnpm-lock.yaml, vite.config.ts, vitest.config.ts, playwright.config.ts
├── components.json             # shadcn config
├── index.html
├── src/
│   ├── main.tsx, App.tsx
│   ├── protocol/
│   │   ├── generated/          # ts-rs output (committed, CI-checked)
│   │   ├── transport.ts        # Transport interface: send(cmd), subscribe(onBatch)
│   │   ├── tauri-transport.ts  # invoke('send_cmd') + Channel
│   │   └── fake-transport.ts   # scripted fixtures for dev and Playwright
│   ├── store/                  # Zustand: ingest(batch) reducer, lists, details, streams, layout
│   ├── layout/                 # width classes, slot layouts, PaneHost, useColumnBudget
│   ├── theme/                  # tokens.css (neon-noir dark / blueprint light), effects.css
│   ├── components/ui/          # shadcn-generated primitives (themed)
│   ├── components/             # ResourceTable, Inspector, DetailSections, LogViewer,
│   │                           #   MetricsChart, ConfirmDialog, CommandPalette, StatusBar
│   ├── resources/              # one descriptor per kind: columns (+priority), detail
│   │                           #   sections, actions, and the commands it uses
│   └── views/                  # Overview, Flux, PortForwards, Integrations, Mcp, Settings
├── tests/
│   ├── unit/                   # Vitest
│   ├── e2e/                    # Playwright specs, one per user story
│   └── fixtures/               # recorded envelope batches (JSON)
└── src-tauri/                  # NEW crate: fathom-desktop
    ├── Cargo.toml              # fathom = { path = "../..", default-features = false }
    ├── tauri.conf.json, capabilities/default.json, icons/
    ├── build.rs
    └── src/
        ├── main.rs             # --only-mcp headless path, else fathom_desktop::run()
        ├── lib.rs              # #[cfg_attr(mobile, tauri::mobile_entry_point)] run()
        └── bridge.rs           # send_cmd, subscribe_events, batching pump thread

pipeline/                       # Dagger: + webkit/node/pnpm deps, ui checks, Tauri build
.github/workflows/              # ci.yml: + ui job; release.yml: tauri-action matrix
tests/                          # gui_*.rs + common/ deleted at parity; backend tests kept
```

**Structure Decision**: This is the standard Tauri layout (`ui/` + `ui/src-tauri/`) inside the existing Cargo workspace. The root `fathom` crate becomes the core library that both binaries share. A cargo feature (`egui-ui`) lets the old and new UIs coexist until parity, and deleting the old UI means removing `src/app`, `src/main.rs`, `tests/gui_*`, and the feature.

**Resource descriptors** are the main way to scale the frontend. The 35 kinds differ mostly in columns, detail sections, and actions, so each kind is one data file in `ui/src/resources/` rendered by the shared `ResourceTable` and `Inspector`. This is what makes SC-007 (a new kind in under half a day) realistic.

## Delivery phases (input for /speckit-tasks)

1. **Foundation**:
   - Add serde + ts-rs to the protocol, with round-trip tests.
   - Add the `egui-ui` feature gate.
   - Scaffold `ui/` + `src-tauri` + bridge.
   - Build the fake transport, the store/reducer, the theme tokens, the width-class layout shell, and CI jobs.
2. **US1 + US2 (P1)**:
   - Context and namespace selection, nav tree, ResourceTable (virtualized, search/sort, column budgets), and Inspector.
   - Pods, then Deployments, as the first two descriptors.
   - Layout profiles.
   - Playwright at phone, wide, and ultra widths.
3. **Descriptor fan-out**: the remaining 33 kinds, written as descriptors (this parallelizes well).
4. **US3 (P2)**: actions and confirmations (delete, bulk delete, scale, restart).
5. **US4 (P2)**: LogViewer (ring buffer, follow), port forwards, overview and metrics charts, and the Flux dashboard.
6. **US5, US6 (P3)**: command palette and shortcuts; Integrations, MCP, and settings.
7. **Cutover**:
   - Port every `tests/gui_*.rs` scenario to Playwright.
   - Switch the release to tauri-action.
   - Delete `src/app`, `src/main.rs`'s GUI path, the egui deps, the `egui-ui` feature, and `tests/gui_*`.

## Risks

| Risk | Mitigation |
|---|---|
| WebKitGTK is slow with large DOM tables on Linux | Always virtualize rows. Keep glitch and scanline effects on chrome only. Performance tests with 10k rows in the WebKit Playwright project. |
| Serde tagging churns the contract and breaks the MCP server | The MCP server uses the Rust types directly, not JSON, so it's unaffected. The round-trip tests pin the representation. |
| Event floods overwhelm the IPC bridge | Coalesce into batches (≤16 ms / 256 envelopes). The reducer applies each batch in one store update. |
| Views reach into backend persistence directly | Move those calls behind protocol commands during Foundation. |
| `ubuntu-22.04-arm` runners aren't available for this repo's plan | Fall back to building on arm64 with a self-hosted runner, or ship aarch64 as deb/rpm only (no AppImage) with cross-compilation. |
| Parity scope creep (MVP.md features not built today) | The spec limits scope to current parity. Applications and Helm views are out. |

## Complexity Tracking

No constitution violations to justify.
