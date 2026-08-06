# Fathom

> A fast, operator-first Kubernetes dashboard — built for people who live in the terminal but want a real UI.

![CI](https://github.com/mbround18/fathom/actions/workflows/ci.yml/badge.svg)

---

## What it is

Fathom is a desktop Kubernetes dashboard written in Rust using [egui](https://github.com/emilk/egui). It gives platform engineers and operators a dense, responsive UI for navigating and managing Kubernetes clusters — without the sluggishness of web-based tools or the context-switching of raw `kubectl`.

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

### Pre-built binaries

Download the latest release for your platform from the [Releases page](https://github.com/mbround18/fathom/releases):

| Platform | Download |
|---|---|
| Linux x86_64 | `fathom-linux-x86_64.tar.gz` |
| Linux aarch64 | `fathom-linux-aarch64.tar.gz` |
| macOS x86_64 | `fathom-macos-x86_64.tar.gz` |
| macOS Apple Silicon | `fathom-macos-aarch64.tar.gz` |
| Windows x86_64 | `fathom-windows-x86_64.zip` |

```bash
# Linux/macOS example
curl -L https://github.com/mbround18/fathom/releases/latest/download/fathom-linux-x86_64.tar.gz | tar xz
./fathom
```

### Build from source

```bash
# Prerequisites: Rust 1.75+, system GTK/xcb dev libs (Linux)

# Ubuntu/Debian
sudo apt-get install -y libgtk-3-dev libxcb-render0-dev libxcb-shape0-dev \
  libxcb-xfixes0-dev libxkbcommon-dev libssl-dev pkg-config

# macOS (Xcode CLI tools are sufficient)

# Build
git clone https://github.com/mbround18/fathom
cd fathom
cargo build --release
./target/release/fathom
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

```bash
# Run in dev mode
cargo run

# Check for errors/warnings
cargo check

# Lint
cargo clippy

# Format
cargo fmt
```

### Architecture

Fathom uses a two-thread model:

- **UI thread**: egui/eframe event loop — never blocks
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
