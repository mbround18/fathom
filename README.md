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
- Abstractions that hide Kubernetes objects (guided setup paths are fine — they show every artifact and confirm every write)
- Multi-cloud abstractions
- Replacing `kubectl` for one-off operations

---

## Installation

### Pre-built bundles

Download the latest release from the [Releases page](https://github.com/mbround18/fathom/releases).
The MVP ships **Linux x86_64 only**, as the Tauri bundler names them:

| Bundle | Asset |
|---|---|
| Debian / Ubuntu | `Fathom_<version>_amd64.deb` |
| Fedora / RHEL | `Fathom-<version>-1.x86_64.rpm` |
| Any distro | `Fathom_<version>_amd64.AppImage` |

Packages are **unsigned**. macOS (`.dmg`) and Windows (`.msi`, NSIS) bundles, code signing and
notarization are planned for after the MVP; aarch64 Linux packages wait until someone has tested
them.

### Build from source

Prerequisites:

| Tool | Version | Notes |
|---|---|---|
| Rust | 1.99.0 | pinned in `rust-toolchain.toml`; rustup picks it up |
| Node | LTS (22+) | |
| pnpm | 9+ | `corepack enable` |
| Linux system libs | webkit2gtk-4.1 | see below |

```bash
# Ubuntu/Debian
sudo apt install libwebkit2gtk-4.1-dev build-essential curl wget file libxdo-dev \
  libssl-dev libayatana-appindicator3-dev librsvg2-dev

git clone https://github.com/mbround18/fathom
cd fathom
pnpm -C ui install

make build-linux            # deb + rpm + AppImage + the binary, all into ./dist
make                        # the full target list (build-windows and build-macos exist but are post-MVP, untested)
make build-windows-paws     # Windows NSIS installer, cross-built in a container via paws (scripts/build-windows.sh)
```

Each target stages into `./dist` beside a regenerated `SHA256SUMS`; the underlying
`pnpm -C ui tauri build` leaves its output under the cargo target dir instead. Bundles
built this way are **unsigned** — signing and notarization are post-MVP.
Third-party licences are listed in `THIRD_PARTY_LICENSES.md`.


### Hosted server mode (in-cluster, browser)

Fathom also runs as a web service inside a cluster: people sign in through your OIDC provider
(Keycloak, Rauthy, Dex, Entra ID, Okta...) and every Kubernetes request is made as the signed-in
person, so cluster RBAC decides what each of them sees. Mutations are recorded in an attributed,
append-only audit log.

```bash
helm install fathom deploy/helm/fathom -n fathom --create-namespace \
  --set externalUrl=https://fathom.example.com \
  --set oidc.issuer=https://idp.example.com/realms/ops --set oidc.clientId=fathom
```

See [docs/HOSTED.md](docs/HOSTED.md) for provider claim mappings, the impersonation model for
managed clusters, ingress/WebSocket notes and the audit log.

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

Fathom ships a stdio MCP server (`src/backend/mcp/`) for same-machine AI agents such as Claude
Code or Claude Desktop. It is **off by default** and has no network listener; enable it from the
app or run it headless:

```
Integrations → MCP → enable          # in the app
cargo run -p fathom-desktop -- --only-mcp   # headless, stdio only
```

Read tools: get and list objects, events, a pod log tail, and a pod health summary. Mutating tools
(scale, replace, delete) ask the agent to confirm with you first and pass the same RBAC check as
the UI. The MCP server is experimental in 0.1.0.

---

## Configuration

Fathom keeps its configuration under `~/.fathom/` (per-context settings, fleet metadata, the
local mutation audit log). Secrets such as Grafana API keys live in the OS keyring, not on disk.

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

# CI, exactly as GitHub runs it (containers via paws + Docker)
scripts/ci.sh                          # all jobs; or: scripts/ci.sh check|test|ui
```

`scripts/ci.sh` is the only thing `.github/workflows/ci.yml` runs, so a green local run is a green
CI run. It needs [`paws`](https://github.com/mbround18/paws) with `paws run`
(`cargo install --locked --git https://github.com/mbround18/paws --branch feat/fathom-local-ci paws-cli`
until that is released) and Docker. It builds from your working tree minus everything `.gitignore`d,
so uncommitted edits are tested but local `target/` and `node_modules/` are not. It is sized for a
shared workstation: `CARGO_BUILD_JOBS=4`, `PLAYWRIGHT_WORKERS=2`, niced, and the `ui` job refuses to
start with less than 8GB of memory available (`CI_MIN_FREE_GB`). Build caches live in the dagger
engine's `fathom-ci-*` cache volumes.

Performance checks: `scripts/measure-idle-cpu.sh` (SC-005) and `scripts/measure-startup.sh` (SC-003).

### Architecture

The `fathom` library crate in `src/` is the backend: every Kubernetes call, integration and the
MCP server run there on a tokio runtime, behind a typed command/event protocol (`src/protocol/`,
exported to TypeScript as `ui/src/protocol/generated/`). `ui/src-tauri` is the Tauri v2 desktop
shell that hosts the React + TypeScript UI in `ui/` and bridges it to the backend. `server/`
(`fathom-server`) serves the same UI and backend as an in-cluster web app with OIDC sign-in, and
`daemon/` (`fathom-daemon`) is the optional in-cluster aggregator for topology, metrics and event
snapshots. Everything the UI shows comes through the protocol, so the shells are thin.

---

## Contributing

PRs welcome. Please run `cargo fmt` and `cargo clippy` before submitting.

---

## License

Apache-2.0 — see [LICENSE](LICENSE).
