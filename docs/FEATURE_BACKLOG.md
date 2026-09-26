# Feature backlog (competitor-mined, 2026-09-23)

Source: mined 15 competitor/plugin repos (Headlamp + 8 plugins, kubenav, Lens, Rancher, k9s,
Devtron) against Fathom's shipped feature set (specs 001-015). Full research is in the
2026-09-23 planning conversation. This file tracks what's **queued but not yet started**
(Tier 4/5 and explicitly-declined items) so it isn't lost once Tier 1-3 specs (016-029) are
in flight.

## In flight now (specs 016-029)

Tier 1 (command palette, Popeye-style linter, proactive RBAC-aware UI, version badge),
Tier 2 (Gatekeeper/Trivy/Velero/cert-manager panes), Tier 3 (pod-exec terminal, ephemeral
debug containers, mutation audit-log viewer, Fleet-style multi-context rollup, Knative pane,
KEDA pane). See `specs/016-*` through `specs/029-*` once created.

## Priority principle: never blackbox the cluster — manifest export (user, 2026-09-26)

Fathom should always get the operator close to the underlying manifests. Anything the app
shows or creates must be exportable as plain Kubernetes YAML, so the cluster never becomes
something only Fathom can explain. Found while using Fathom's RBAC UI to set up a scoped
prod ServiceAccount: the objects were created, but getting the equivalent manifests back
out had to happen outside the app.

- **Priority — per-resource export**: a Copy YAML / Download YAML action on every inspector
  and resource row. Offer a "clean for re-apply" option that strips `status`,
  `managedFields`, `resourceVersion`, `uid` and `creationTimestamp`. Today YAML is only
  reachable through the edit/apply flow.
- **Priority — multi-select / bundle export**: export a selection, a namespace, or an
  Application (004 attribution) as one multi-document YAML file.
- **Priority — "show me the manifest" before every mutation**: every create/edit flow
  (RBAC 011, Helm 006, node shell 037, debug containers 025, kind provisioning 030) exposes
  the exact manifest it will apply, copyable, next to the existing diff preview.
- **Later (huge boon, not a priority)**: generate a Helm chart (Chart.yaml + templated
  values) or a kustomize base/overlay from a selection. Needs a templating-heuristics
  decision (which fields become values), so spike it before spec-kit.

## Tier 4 — needs an architecture spike before spec-kit

- **Plugin/extension API** — the mechanism behind Lens's marketplace and every Headlamp
  plugin surveyed. Turns Fathom into a platform instead of one vendor's GUI. Needs a
  sandboxing-model decision (WASM vs. isolated webview vs. Rust dylib) before any spec-kit
  work — don't spec this cold.
- **Kubescape pane** — broader than Gatekeeper/Trivy (compliance + runtime threat +
  policy *authoring*, not just visualization). Bigger mutation surface than the Tier 2
  security panes; do after they land and the "Security & Policy pane family" pattern is
  proven.
- **KubeVirt-lite** — VM list + status + gated start/stop/migrate/delete only (no console).
  Full KubeVirt plugin scope (VNC/serial console, live migration, DataVolume management) is
  a new architecture-level capability class (interactive framebuffer/terminal streaming, not
  request/response) — scope the "-lite" version only, and only after the pod-exec terminal
  (spec in Tier 3) ships, since it establishes the interactive-stream pattern this would
  reuse.

## Tier 5 — lower priority / niche polish

- Tray/menu-bar quick-glance cluster health widget + native OS notifications for watched
  resource events (crashloop, OOMKill, NodeNotReady) — kubenav-inspired, genuinely novel for
  desktop K8s GUIs, wires `SystemEvent` to `tauri-plugin-notification`, but not urgent.
- Cluster catalog/favorites hotbar (pinned contexts, custom icons, per-entry health badge) —
  small UX polish on top of the existing federation switcher (spec ~012).
- Local chart install from a local `.tgz`/directory — small Helm-scope extension, adjacent
  to but out of scope for 006-helm-mutate.
- Unified "App Details" debug view compositing logs+events+metrics on a single non-Ready pod
  — check first whether the Inspector already composites this before speccing; may be a
  no-op.

## Reversed decision: in-app local KinD cluster provisioning (now spec 030)

Originally declined below as "cluster lifecycle/provisioning" on the grounds that Fathom
federates arbitrary kubeconfig contexts transparently and doesn't need a provisioning UI.
The user explicitly asked (2026-09-23) for local KinD provisioning specifically, to make
spinning up a disposable local cluster (for trying out or validating the Tier 1-3 features
above) a first-class, in-app, zero-terminal action — "hyper powerful" as a differentiator,
not just parity. This is narrower than the declined item: **KinD only, localhost only**, not
general remote cluster lifecycle/provisioning (still declined) and not Rancher-style Projects
state (still declined, see below). See `specs/030-kind-cluster-provisioning/`. Distinct from
the separate `scripts/dev-cluster` KinD harness (dev/CI tooling, not product UI) already in
flight — the two should share install-profile logic where sensible rather than duplicating it.

## Queued follow-on: in-app CLI toolchain manager (future spec, after 030 lands)

User's idea (2026-09-23), a natural sibling to spec 030's "hyper powerful" in-app cluster
provisioning: a setup panel that manages the CLI binaries Fathom's features depend on or
commonly pair with — `kubectl`, `helm`, `kind`, and similar — from inside the app instead of a
terminal. Scope sketch for whenever this is speckit'd:

- Detect what's already on the host (version + path) for each tool, so the panel shows
  "already installed" vs. "not found" per binary rather than assuming a clean machine.
- Let the operator install a specific version, or switch/pin to a different installed version,
  per tool — not just "latest."
- Installs land under the user's own `~/.local/bin/` (standard user-writable location, no sudo,
  no system package manager involvement) — consistent with how spec 030 should treat a
  vendored `kind` binary if it goes that route (see spec 030's research.md decision on that).
- Likely shares its "detect an external binary, offer to fetch/pin a version, write into
  `~/.local/bin/`" plumbing with spec 030's own kind/container-runtime detection — worth
  designing as one small shared backend module both specs can use rather than two copies, once
  030 has landed and this is picked up.
- Not yet spec-kit'd; do this after spec 030 ships so the shared plumbing has something real to
  factor out of, rather than guessing at the abstraction upfront.

## Explicitly declined (re-evaluated, not planned)

- Literal JS plugin runtime — wrong shape for a compiled Tauri app.
- Rancher Projects (namespace grouping/quota) — would require persisted state outside the
  cluster, conflicting with "backend owns the cluster, no shadow state."
- General remote cluster lifecycle/provisioning (cloud-provider cluster creation, minikube
  bridging) — Fathom already federates arbitrary kubeconfig contexts transparently and
  doesn't need a provisioning UI for clusters it doesn't run locally. (Local KinD provisioning
  is the one carved-out exception — see above.)
- Mobile-only patterns (biometric lock, QR cluster import) — no mobile client.
- k9s-style benchmark/load-testing via port-forward — first "active traffic generation"
  dependency category, niche, defer indefinitely.
- Devtron's CI/build/SSO/DORA-metrics surface — orthogonal to "GUI onto a live cluster."

## After Tier 1-3 ship

Promo brochure v2 refresh (plan already drafted: competitor comparison page, light-theme
content pages, live Flux + Hubble/Cilium panel screenshots) — re-run once the new panes
(especially the Security & Policy family and GitOps/observability family) are shippable and
screenshot-worthy, so the brochure showcases the newly-shipped work too, not just Flux/Hubble.
