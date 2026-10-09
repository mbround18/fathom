# Changelog

All notable changes to Fathom are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.0] - Unreleased

First release: a desktop Kubernetes dashboard for Linux. Rust backend, Tauri v2 shell, React UI.
The feature list below is the high-level shape; `docs/MVP.md` tracks each area.

### Added

- Cluster overview with resource summaries, utilisation series and warning events; a navigator
  covering nodes, workloads, network, storage, config, access control, custom resources,
  namespaces and events, with cross-links between resources and a command palette.
- Workloads: list, inspect, scale, restart and delete; pod lifecycle, probes and init-container
  diagnostics; streaming logs; interactive exec with a confirmation and RBAC pre-check; ephemeral
  debug containers; container image layer inspection.
- YAML edit, diff and confirm-gated apply for any resource; manifest export (per resource and as
  bundles, cleaned for re-apply), kubeconfig export.
- Node actions: cordon, uncordon, drain, taint; a cluster version badge.
- Applications view with Helm release linking; Helm rollback, upgrade and uninstall.
- Topology graph (namespace to pod), a read-only cluster linter, a fleet rollup dashboard and a
  fleet model (named, labelled, groupable clusters).
- Port forwards: create, list, stop, persisted across sessions.
- Metrics from Prometheus or metrics-server, with auto-connect through the API server proxy;
  Grafana deep links.
- Integration panes: Flux (GitOps), KEDA, Hubble/Cilium, Gatekeeper, Trivy Operator, Velero,
  cert-manager, Knative, Longhorn.
- Flux verbs from the inspector (`049-flux-mutate`): suspend, resume and reconcile (with source)
  on sources, Kustomizations and HelmReleases; the confirmation names the object and what the verb
  reaches; every verb is audited and allowed in hosted mode.
- Access control: browse, create, edit and delete Roles, ClusterRoles and bindings with a rules
  editor and presets; namespace-access lookup; guided OIDC setup; RBAC-aware UI that disables
  what the identity cannot do; a local mutation audit-log viewer.
- Setup flow for first run: pick a context, connect, sign in, or create a local kind cluster from
  inside the app.
- Cascading wide-screen layout; responsive down to phone width.
- Hosted server mode (`fathom-server` plus Helm chart): browser UI inside a cluster, OIDC sign-in,
  every request made as the signed-in person, attributed append-only audit log.
- In-cluster daemon (`fathom-daemon`) for aggregated topology, metrics and event snapshots across
  contexts; simultaneous multi-context federation.
- Managed-cloud exec-credential plugins (EKS, GKE, AKS).
- Node Shell: an on-demand, confirm-gated privileged debug pod on a node with guaranteed cleanup.
- Docker management for the local machine: containers, Compose projects, images, volumes and
  networks, read-only until a project is adopted, typed confirmation on destructive verbs.
- Agent SDK operations (AI-assisted troubleshooting, bring-your-own key or local Ollama), SDK
  sign-in (GitHub device flow, Claude and OpenAI keys in the OS keyring), and a stdio MCP server
  (`src/backend/mcp/`) that is off by default and exposes read tools plus confirm-gated scale,
  replace and delete.
- Teams and organisations: shared non-secret connection profiles and org-level settings sync.
- Observability settings persisted under `~/.fathom/`, secrets in the OS keyring.

### Packaging

- Linux bundles: `.deb`, `.rpm` and `.AppImage` for x86_64, built by Tauri 2. The AppImage is
  repacked without the bundled `libwayland-client` (`scripts/appimage-host-libs.sh`), which
  otherwise blanks the window on current WebKitGTK.
- The deb control file carries `Section: utils` and a maintainer with an email address.
- Deb package name: the control file says `Package: fathom`, which collides with Ubuntu's `fathom`
  package (a chess tablebase tool); `apt install ./Fathom_*.deb`
  on a machine with that package installed would replace it. Tauri 2 derives the deb `Package`
  field from `productName` (kebab-cased) and its `bundle.linux.deb` block has no name override,
  so renaming it to `fathom-k8s` without renaming the product is not possible from
  `tauri.conf.json`. Until the bundler gains that knob or the package is post-processed, install
  with that caveat in mind.
- Release builds use thin LTO, one codegen unit and stripped symbols; panics still unwind
  because the backend relies on `catch_unwind`.
- `THIRD_PARTY_LICENSES.md` lists every bundled crate and production npm package with its
  licence; regenerate with `scripts/third-party-licenses.sh`.

### Known limitations

- Linux only. macOS and Windows bundles, code signing and notarisation come after this release.
- Packages are unsigned.
- aarch64 Linux packages are not built: nobody has tested them yet.
- Experimental, validated against unit and mock tests or a local kind cluster rather than live
  targets: AI assistant and MCP server, Docker management, fleet federation, the in-cluster
  daemon, Teams and organisations, managed-cloud auth plugins (EKS/GKE/AKS), Node Shell.
- Hosted server mode has open validation items (25-user soak, Helm upgrade/uninstall check,
  WebKit pass); see `docs/MVP.md`.
- CI runs on manual dispatch only until the full suite is green twice in a row.

[0.1.0]: https://github.com/mbround18/fathom/releases/tag/v0.1.0
