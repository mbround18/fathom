# Fathom

> A fast, operator-first Kubernetes dashboard — built for people who live in the terminal but want a real UI.

![CI](https://github.com/mbround18/fathom/actions/workflows/ci.yml/badge.svg)

---

## What it is

Fathom is a desktop Kubernetes dashboard built with a Rust backend and a [Tauri](https://tauri.app) + React UI. It gives platform engineers and operators a dense, responsive UI for navigating and managing Kubernetes clusters — without the sluggishness of web-based tools or the context-switching of raw `kubectl`.

**Goals:**
- Single-pane observability: nodes, workloads, network, storage, access control, events — all in one app
- Real Prometheus metrics, not just resource requests/limits
- Port-forwarding, log streaming, scaling, and restarts without leaving the UI
- MCP (Model Context Protocol) server for AI-agent integration
- Sub-200ms UI interactions, even on large clusters

**Non-goals:**
- Beginner onboarding / wizards
- Multi-cloud abstractions
- Replacing `kubectl` for one-off operations

---

## Installation

### Pre-built bundles

Download the latest release for your platform from the [Releases page](https://github.com/mbround18/fathom/releases). Assets are named `fathom-<version>-<os>-<arch>.<ext>`:

| Platform | Bundles |
|---|---|
| Linux x86_64 / aarch64 | `.deb`, `.rpm`, `.AppImage` |
| macOS Apple Silicon / Intel | `.dmg` |
| Windows x86_64 | `.msi`, `.exe` (NSIS) |

### Build from source

Prerequisites:

| Tool | Version | Notes |
|---|---|---|
| Rust | 1.85+ (edition 2024) | |
| Node | LTS (22+) | |
| pnpm | 9+ | `corepack enable` |
| Linux system libs | webkit2gtk-4.1 | see below |
| macOS | Xcode CLT | |
| Windows | WebView2 runtime + MSVC build tools | WebView2 ships with Windows 11 |

```bash
# Ubuntu/Debian
sudo apt install libwebkit2gtk-4.1-dev build-essential curl wget file libxdo-dev \
  libssl-dev libayatana-appindicator3-dev librsvg2-dev

git clone https://github.com/mbround18/fathom
cd fathom
pnpm -C ui install
pnpm -C ui tauri build      # host-platform bundles under the cargo target dir's release/bundle/
```

---

## Requirements

- A `~/.kube/config` (or `KUBECONFIG` env var) pointing to a reachable cluster
- Kubernetes 1.25+
- RBAC: `get`/`list`/`watch` on the resources you want to view; `update`/`patch` for mutations

---

## Features

### Navigation

- Left-hand tree navigation: Overview · Nodes · Workloads · Network · Storage · Config · Access Control · Events
- Global namespace filter (top bar) — filters all views simultaneously
- Clickable cross-links between resources (pod → workload, namespace label → filter)

### Workloads

| Resource | List | Details | Scale | Restart | Delete | Metrics |
|---|---|---|---|---|---|---|
| Pods | ✅ | ✅ | — | — | ✅ | ✅ |
| Deployments | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| DaemonSets | ✅ | ✅ | — | ✅ | ✅ | ✅ |
| StatefulSets | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| ReplicaSets | ✅ | ✅ | ✅ | ✅ | ✅ | — |
| Jobs | ✅ | ✅ | — | — | ✅ | — |
| CronJobs | ✅ | ✅ | — | — | ✅ | — |

### Network

- Services, Endpoints, Ingresses, IngressClasses, NetworkPolicies
- **Port Forwards**: create, list, stop, persist across sessions

### Storage

- PersistentVolumes, PersistentVolumeClaims, StorageClasses
- Disk usage via Longhorn or Prometheus integration

### Config & Access Control

- ConfigMaps, Secrets (value masking), ResourceQuotas, LimitRanges
- Roles, RoleBindings, ClusterRoles, ClusterRoleBindings

### Observability

- **Cluster Events**: real-time warning and normal events
- **Pod Logs**: real-time streaming, container selection, pause/resume
- **Metrics** (requires Prometheus or metrics-server):
  - Node CPU/memory
  - Pod CPU/memory (per-container breakdown)
  - Workload aggregate CPU/memory

### Integrations

#### Prometheus

Fathom auto-discovers Prometheus in your cluster or can be configured manually:

```
Integrations → Metrics → Prometheus
  Namespace: monitoring
  Service:   prometheus-operated
  Port:      9090
```

With Prometheus configured, workload details panels show CPU/memory graphs with real data.

#### Grafana

Link Fathom to your Grafana instance for dashboard deep-links:

```
Integrations → Grafana
  Base URL: https://grafana.example.com
  API Key:  <your API key>
```

#### MCP (Model Context Protocol)

Fathom ships an MCP server for AI-agent integration (e.g. Claude, Cursor):

```
Integrations → MCP
  → Start Server
```

This exposes cluster state as MCP tools — query namespaces, pods, deployments, and trigger restarts from your AI coding assistant.

---

## Configuration

Fathom stores configuration per kubeconfig context at:
- Linux/macOS: `~/.config/fathom/<context>.json`
- Windows: `%APPDATA%\fathom\<context>.json`

Settings include: Prometheus endpoint, Grafana URL, MCP auto-start, port-forward history.

---

## Development

Commands run from the repo root (full detail in `specs/001-tauri-shadcn-frontend/quickstart.md`).

```bash
pnpm -C ui install
cargo test -p fathom export_bindings   # generates ui/src/protocol/generated/

# Run
pnpm -C ui tauri dev                   # full app: Vite HMR + Rust backend
pnpm -C ui dev:fake                    # browser only, fake transport (no cluster needed)
cargo run -p fathom-desktop -- --only-mcp   # headless MCP server

# Test
pnpm -C ui lint && pnpm -C ui typecheck
pnpm -C ui test                        # Vitest
pnpm -C ui e2e                         # Playwright: chromium + webkit x phone..ultra
cargo test --workspace                 # backend, serde round-trips, bindings drift

# Rust checks
cargo check
cargo clippy
cargo fmt
```

Performance checks: `scripts/measure-idle-cpu.sh` (SC-005) and `scripts/measure-startup.sh` (SC-003).

### Architecture

Fathom pairs a Rust backend with a Tauri v2 webview UI (`ui/`):

- **UI**: React webview hosted by the Tauri shell (`ui/src-tauri`) — never blocks the backend
- **Backend thread**: tokio runtime — all Kubernetes API calls and async work

Communication uses crossbeam channels:
- `BackendCmd` (UI → backend): commands like `RefreshPods`, `ScaleDeployment`
- `BackendEventEnvelope` (backend → UI): events like `PodsUpdated`, `PodDetailsUpdated`

Protocol types live in `src/protocol/`. View components implement `ViewComponent` trait in `src/app/views/`.

---

## Contributing

PRs welcome. Please run `cargo fmt` and `cargo clippy` before submitting.

---

## License

MIT
