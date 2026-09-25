# Quickstart: Tauri + shadcn Frontend

**Plan**: [plan.md](./plan.md)

This is how to run, test, and validate the new frontend once Foundation tasks are done. Commands assume the repo root.

## Prerequisites

| Tool | Version | Notes |
|---|---|---|
| Rust | ≥ 1.85 (edition 2024) | already required |
| Node | LTS (22+) | |
| pnpm | 9+ | `corepack enable` |
| Linux system libs | webkit2gtk-4.1 | `sudo apt install libwebkit2gtk-4.1-dev build-essential curl wget file libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev` |
| macOS | Xcode CLT | |
| Windows | WebView2 runtime + MSVC build tools | WebView2 ships with Windows 11 |
| A kubeconfig | at least one reachable context | kind/k3d is fine |

## First-time setup

```bash
pnpm -C ui install
cargo test -p fathom export_bindings   # generates ui/src/protocol/generated/
```

## Develop

```bash
pnpm -C ui tauri dev                   # full app: Vite HMR + Rust backend
pnpm -C ui dev:fake                    # browser only, fake transport (no cluster needed)
cargo run --features egui-ui           # old UI, until cutover
cargo run -p fathom-desktop -- --only-mcp   # headless MCP, unchanged behavior
```

`dev:fake` sets `VITE_TRANSPORT=fake` and replays `ui/tests/fixtures/*.json`. Use browser devtools' responsive mode to check every width class.

## Test

```bash
pnpm -C ui lint && pnpm -C ui typecheck
pnpm -C ui test                        # Vitest: reducer, column budget, width classes, descriptors, contrast
pnpm -C ui e2e                         # Playwright: chromium + webkit × phone/laptop/wide/ultra
cargo test --workspace                 # backend + serde round-trips + bindings drift
cargo run -p pipeline -- test          # full CI gate (Dagger)
```

## Build

```bash
pnpm -C ui tauri build                 # host-platform bundles in ui/src-tauri/target/release/bundle/
pnpm -C ui tauri icon ../assets/icon.png
```

## Validation scenarios (map to spec user stories)

| # | Story | Steps | Expected |
|---|---|---|---|
| V1 | US1 | Launch `tauri dev`, pick a context, go to Pods | List populated in < 3 s. Search filters as you type. Clicking a row shows the inspector (details, YAML, events). |
| V2 | US1 | Switch context mid-load | No rows from the previous context ever appear. |
| V3 | US2 | Resize the window from 390 px to 5120 px (or use `e2e --project=*`) | Slots go 1→2→3→3+dock→4→5, and never go back down. No horizontal scrollbar. Columns appear in priority order. Search text and selection survive every step. |
| V4 | US2 | On `wide`, pin Deployments in list B, restart the app | The pinned layout is restored for that width class only. |
| V5 | US3 | Select 3 pods → Delete | The dialog lists all 3. Cancel sends nothing. Confirm shows a toast, and the rows disappear after the refresh. |
| V6 | US3 | Scale a deployment to 0, then back | The replica count updates. A failure shows the backend error text. |
| V7 | US4 | Open logs on a chatty pod with follow on | Smooth at ≥ 1k lines/s. Scrolling up pauses follow, and End resumes it. |
| V8 | US4 | Start a port forward, restart the app | The forward is restored from `ports.json` like today. |
| V9 | US5 | Press `Ctrl/Cmd+K`, type "deploy", Enter | Jumps to Deployments. Every action is reachable without a mouse. The focus ring is always visible. |
| V10 | US6 | Toggle the magenta accent, the light blueprint mode, glitch on, and effects off; turn on OS reduced motion | Takes effect instantly and persists. With reduced motion, glitch and other animations don't run. With effects off, there is no glow, scanlines, or grain. |
| V11 | US6 | Toggle MCP auto-start, restart | The same flag as the egui build (`config.json → auto_start.mcp_http`). The MCP server answers on :18288. |
| V12 | SC-009 | Run the new build against an existing `~/.fathom/` from the egui build | Ports, metrics, integrations, and nav groups all load. No prompt appears. |
| V13 | SC-005 | Leave the app idle on Pods for 5 min | < 2% of one core, and the bridge thread is blocked in `recv`. |

V13 procedure (Linux): launch the release build, open Pods, then run `scripts/measure-idle-cpu.sh --attach <pid> --settle 10 --window 300` (it sums utime+stime from `/proc/<pid>/stat` for the app and its WebKit children and prints the percentage of one core; exit status 1 at or above 2%). Without `--attach` the script launches the binary itself and measures the launch view, which is not the Pods view. The automated `pnpm -C ui test:driver` (`ui/tests/driver/idle.test.ts`, 60 s window; prerequisites listed at the top of `ui/tests/driver/harness.ts`) opens Pods and asserts < 2%. On macOS and Windows, watch Activity Monitor or Task Manager, or `Get-Counter`, for the same 5 minutes.

## Live e2e (real backend, real cluster, `fathom-dev` only)

An opt-in Playwright suite (`ui/tests/live/`, config `ui/playwright.live.config.ts`) runs the real UI against the real Rust backend through `tools/devbridge`. It is never part of `pnpm e2e`. Every write is confined to the `fathom-dev` sandbox namespace, both by the bridge's write guard and by the fenced `kubectl()` helper the specs use.

```bash
FATHOM_SANDBOX_CONTEXT=<ctx> scripts/sandbox.sh up       # once: namespace + workloads (dev/sandbox/)
FATHOM_LIVE_CONTEXT=<ctx> pnpm -C ui e2e:live            # builds + starts the devbridge, serves dist-live on :4174
FATHOM_SANDBOX_CONTEXT=<ctx> scripts/sandbox.sh reset    # back to the committed baseline if a run was interrupted
FATHOM_SANDBOX_CONTEXT=<ctx> scripts/sandbox.sh down     # remove the sandbox
```

- `FATHOM_LIVE_CONTEXT` is required and never defaulted. The global setup refuses to start unless `fathom-dev` in that context carries the `fathom.dev/sandbox=true` label.
- The bridge gets a random token for each run, and the specs get it through the `app` fixture. Nothing is written to disk.
- `browse.spec.ts` is read-only. `actions.spec.ts` scales, restarts and deletes sandbox workloads and throwaway pods labelled `fathom.dev/e2e=true`, and `restoreBaseline()` runs after each spec.
- To run one layout only, add `-- --project=chromium-wide` (or `chromium-phone`).
