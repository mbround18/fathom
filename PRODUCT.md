# Product

<!-- impeccable:product-schema 1 -->

## Platform

web

## Users

**Primary: the solo platform engineer operating their own clusters.** Someone who lives in a
terminal and reaches for `kubectl` by reflex, but wants a real UI for the things a terminal is bad
at — scanning a fleet, watching logs stream, reading a topology, comparing what a cluster actually
accepts against what it should. They are fluent in Kubernetes objects and are not being protected
from them.

**Also served, and never traded away: the platform or SRE team operating clusters on behalf of
other engineers.** Their needs — attribution, an audit trail, RBAC correctness, review-before-write
— are not a second product. When the operator's speed and the team's guarantees conflict, speed
yields: the hosted multi-user model depends on those guarantees holding everywhere.

The usage scene to design against is **the desktop app first**; hosted mode inherits those
decisions rather than driving them. (Consistent with the Linux-only MVP.)

## Product Purpose

A fast, operator-first Kubernetes dashboard: single-pane observability and management across nodes,
workloads, network, storage, config, access control and events, with real Prometheus metrics rather
than resource requests and limits standing in for them.

Success is that an operator stops context-switching — port-forwards, log streams, scaling,
restarts, exec, YAML edit-and-apply all happen without leaving the app — and that the app never
feels slower than the terminal it replaced. The stated interaction budget is **sub-200ms, including
on large clusters**.

## Positioning

Parity with Rancher, Lens and the OpenShift console, but opinionated, fast, and composable — and
specifically **not** an abstraction layer. Fathom shows the Kubernetes object. Guided setup paths
are allowed, and are held to a harder standard than a wizard usually is: they display every
artifact they will write and confirm every mutation before making it.

Two things a neighbouring tool could not truthfully copy without building them:

- **It ships as both a desktop app and an in-cluster web service from one codebase**, where the
  hosted mode makes every Kubernetes request *as the signed-in person*, so cluster RBAC — not an
  application role model — decides what each of them sees.
- **It treats the control plane as inspectable and, where Fathom provisioned it, writable** — with
  ownership a recorded fact rather than an inference, a backup taken first, and a verified rollback.

## Operating Context

- Needs a reachable `~/.kube/config` (or `KUBECONFIG`) and Kubernetes 1.25+. RBAC floor:
  `get`/`list`/`watch` to view, `update`/`patch` to mutate.
- Config is stored per kubeconfig context (`~/.config/fathom/<context>.json`), so a single operator
  moving between clusters keeps per-cluster settings.
- Integrates with what is already in the cluster rather than requiring its own stack: Prometheus
  (auto-discovered or manual), Grafana deep-links, Longhorn, Flux, Cilium/Hubble, KEDA, Knative,
  Gatekeeper, Trivy, Velero, cert-manager.
- Ships an MCP server, so cluster state is addressable by AI agents as well as by people.
- Hosted deployments run in-cluster via a Helm chart, behind an OIDC provider (Keycloak, Rauthy,
  Dex, Entra ID, Okta), with pass-through and impersonation access models and an attributed,
  append-only audit log.
- Clusters can be provisioned locally through kind, which is what makes control-plane writes
  legitimate on a cluster Fathom owns.

## Capabilities and Constraints

- **Shipped breadth** is wide and documented per-resource in `README.md` and `docs/MVP.md`: nodes,
  the seven workload kinds with scale/restart/delete where Kubernetes allows it, network including
  persistent port-forwards, storage, config with Secret masking, full RBAC browse and authoring,
  events, pod logs, pod exec, ephemeral debug containers, YAML edit/diff/apply on any resource
  including CRDs, topology graph, cluster linter, Helm read and mutate, manifest export.
- **Non-goals, explicitly:** abstractions that hide Kubernetes objects; multi-cloud abstraction
  layers; replacing `kubectl` for one-off operations.
- **Mutations are diff-previewed and confirmed.** This is a project principle, not a feature of one
  screen.
- **A denied or impossible read is a successful, legible state** — reported verbatim with its
  reason, never a blocked flow and never an invented value. Namespace-only users see "Not
  permitted" rather than an error toast.
- **Manifest export everywhere is a priority**; the cluster is never a black box. Helm-chart
  generation is a nice-to-have by comparison.
- **MVP ships Linux only** — unsigned `.deb`/`.rpm`/`.AppImage`. macOS, Windows, cross-compilation
  and signing are post-MVP, though the codebase targets all three.
- **Known open items** (recorded so they are not mistaken for finished): live cloud-auth verification, a 25-user
  hosted soak, a Helm upgrade/uninstall check and a WebKit pass are written but not run.

## Brand Commitments

- Name: **Fathom**. Tagline in use: *"A fast, operator-first Kubernetes dashboard — built for
  people who live in the terminal but want a real UI."*
- A committed visual world already exists in code under `ui/src/theme/` (`tokens.css`,
  `effects.css`, `BrandMark.tsx`, `CircuitTrace.tsx`). It is not yet written down — `document`
  would record it. Future work treats it as the incumbent authority, not as a blank slate.
- Voice, as the shipped copy establishes it: plain, specific, and willing to say what is not known.
  UI text names the thing that happened and the reason for it rather than reassuring.

## Evidence on Hand

- Real: the shipped application and its 45 numbered specs under `specs/`, a promo screenshot suite
  (`ui/playwright.promo.config.ts`), an operator guide at `docs/HOSTED.md`, a security review at
  `specs/031-hosted-server-mode/security-review.md`, and measured performance figures in
  `docs/MVP.md` (e.g. first pod list 810 ms via the daemon vs 1,311 ms direct at 5k pods).
- Live validation that genuinely ran: hosted Chromium login/RBAC/revocation on Rauthy and Keycloak
  (2026-09-26); the 045 control-plane read and write against real kind clusters.
- **Absent, and not to be fabricated:** customers, testimonials, case studies, press, adoption
  numbers, pricing, licensing terms and any uptime or scale claim beyond the measured figures
  above.

## Product Principles

1. **Never a black box.** Every view can show the underlying object, and every write can show the
   manifest it will produce. Guided paths raise the bar for disclosure instead of lowering it.
2. **The operator's time is the scarce resource.** Density, scanability and sub-200ms interaction
   beat decoration; a screen that makes someone wait has failed regardless of how it looks.
3. **Say what is true, including "I don't know."** Unreadable, denied and partial states are
   first-class and carry the system's own words.
4. **Earn a mutation before making it.** Back up, preview, confirm, verify it landed, and be able
   to roll back. Ownership and permission are recorded facts, never inferred.
5. **One product, two deliveries.** Desktop leads, but nothing may be designed that cannot hold up
   as a multi-user, RBAC-scoped surface in a browser.

## Accessibility & Inclusion

**WCAG 2.2 AA is the floor**, not an aspiration: contrast ratios, visible focus, full keyboard
reachability and correct names/roles are auditable requirements. This binds hardest on a dense dark
theme, where low-contrast "subtle" greys are the easiest mistake to make — and on an operator tool
whose users are keyboard-first by habit, so keyboard completeness is a usability requirement as
much as an accessibility one.
