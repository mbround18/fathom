# Fathom Constitution

<!--
Sync Impact Report
- Version change: 1.2.0 -> 1.3.0 (MINOR: scope extended to Keel, new platform tier, stack
  additions)
- Modified sections:
  - Scope: Keel (Ansible from source control) joins Fathom and Depths.
  - Principle II: Keel's protocol crate `crates/keel-protocol` named. Its public API reuses the
    same protocol over HTTP; receivers and runners are separate documented contracts.
  - Principle VI: Keel's data added: insert-only hash-chained audit, immutable inventory
    revisions, imports that never overwrite, credentials never leave sealed storage.
  - Supported Platforms: new "Keel server (Linux container)" tier, with a separate runner
    service.
  - Stack: PostgreSQL through sqlx (Keel), and ansible-core in the Keel runner image only.
- Principles: none removed or redefined; every existing MUST still holds for Fathom and Depths.
- Templates requiring updates: none (plan/spec/tasks templates carry no app name)
- Affected specs: specs/055-ansible-repo-control-plane (plan's scope gate satisfied)
- Follow-up TODOs: none
-->

## Scope

The orchestration companion suite is Fathom (Kubernetes), Depths (Docker hosts) and Keel (Ansible
runs driven from source control), plus the
shared crates in `crates/` and packages in `packages/`. The principles below apply to every app
in the suite. Where a rule names Kubernetes, kubeconfig or `~/.fathom/`, read the equivalent for
the app in question (for Depths: the Docker host, the stacks root and its data directory; for Keel: the managed
nodes, its database and its data directory).

## Core Principles

### I. Operator-First, Explicit Mutation

Fathom is built for operators who already understand Kubernetes. Its job is to make cluster state
visible and to act on it only when the operator explicitly asks.

- Every mutating action (delete, bulk delete, scale, restart, port-forward changes) MUST pass
  through an explicit confirmation that names every affected object by namespace/name.
- The UI MUST NOT change the cluster optimistically or implicitly. It shows a resource as changed
  only after the backend reports the result.
- Backend errors MUST reach the operator verbatim, not replaced with generic messages.
- Wizards, onboarding flows, and abstractions that hide Kubernetes objects are out of scope.

**Rationale**: A dashboard that mutates clusters silently is worse than `kubectl`. Trust depends
on showing what's happening before and after acting.

### II. One Contract Per App, Backend Owns the Runtime

- All cluster or Docker access, persistence, and long-running work (watches, log streams, port
  forwards, metrics, compose operations, the MCP server) MUST live in the app's Rust backend.
- Each app owns one versioned command/event protocol (Fathom: `src/protocol/`; Depths:
  `crates/depths-protocol`; Keel: `crates/keel-protocol`). Its UI MUST talk to its backend only through that protocol. UI code
  MUST NOT call the backend's internals or read or write the app's on-disk state directly.
- The protocol MUST be defined once, in Rust. Any other client (the TypeScript UI, or future mobile
  shells) MUST use types generated from it. A breaking change on one side MUST fail the build
  or tests, not show up at runtime.
- A domain shared by several apps (for example, container runtime types) MUST define its wire
  types once, in a shared crate that each app's protocol re-exports. Moving a type into a shared
  crate MUST NOT change the generated bindings of an existing app.
- Protocol changes that break existing clients MUST bump the protocol version.

**Rationale**: A single contract lets the UI technology change (egui → Tauri → mobile) without
touching cluster logic, and keeps the MCP server and the UI behaving the same.

### III. Tested at the Boundaries (NON-NEGOTIABLE)

- Every user story MUST have automated end-to-end tests that run the UI against a scripted fake
  backend. They drive the UI by injecting events and asserting the emitted commands.
- Every protocol command and event variant MUST have a serialization round-trip test. Generated
  bindings MUST be checked for drift in CI.
- UI logic that decides what the operator sees (search, sort, column priority, layout selection,
  event staleness) MUST have unit tests.
- A behavior change MUST come with a test that fails without it. Removing a test requires
  replacing its coverage.
- CI MUST be green before merge. That means `cargo fmt --check`, `cargo clippy -- -D warnings`,
  `cargo test`, and, for the frontend, lint, typecheck, unit tests, and E2E tests.

**Rationale**: The UI has been rewritten before. Boundary tests carry across rewrites, and they
are the only proof that each rewrite kept parity.

### IV. Density Scales With Width

- Layouts MUST be designed mobile-first. Each wider width class MUST add information
  (panes, columns, persistent tool areas) and MUST NOT just enlarge what is already there.
  The number of visible panes MUST NOT decrease as width increases.
- The page MUST NOT scroll horizontally at any supported width. Table columns have priorities and
  are hidden (never squeezed unreadably). Hidden values stay reachable in the inspector.
- UI state (selection, search text, scroll position) MUST survive layout changes.
- Everything MUST be operable by touch (at narrow widths), mouse, and keyboard alone. Text and
  essential UI MUST meet WCAG 2.2 AA contrast. Decorative theme effects MUST be optional, and
  MUST respect the OS reduced-motion setting.

**Rationale**: The same app should be usable on a phone and should show "most of the app at
once" on an ultrawide monitor.

### V. Fast and Quiet

- With a 10,000-object list receiving continuous updates, user input MUST be handled in under
  100 ms. Routine interactions SHOULD complete in under 200 ms.
- An idle app MUST use under 2% of one CPU core. Redrawing continuously when nothing has
  changed is a defect.
- Large lists and log views MUST be virtualized. High-volume streams MUST be batched and bounded
  (ring buffers with explicit truncation markers).
- Events from a previous context or a superseded request MUST be discarded, never rendered.
- A change that regresses startup time to first populated list relative to the recorded
  baseline MUST be justified in its PR.

**Rationale**: Speed is Fathom's main advantage over web dashboards, and a desktop app that burns
CPU while idle does not get kept installed.

### VI. User Data Is Sacred

- Existing on-disk state in `~/.fathom/` (port forwards, metrics settings, integrations,
  credentials, config) MUST keep loading across upgrades with no manual steps. Format changes MUST
  be additive, or be migrated automatically with the original preserved.
- Unreadable or invalid config MUST fall back to defaults without deleting the user's file.
- Secrets and credentials MUST NOT be logged, exported through the protocol to places that
  display them, or shown in the UI unless the user explicitly reveals them.
- The UI and the MCP server MUST respect the same kubeconfig context and RBAC as the user. Fathom
  never escalates privileges.
- Depths treats the stacks root as the only source of truth for stacks: it writes compose and
  `.env` files byte for byte, never rewrites files it was not asked to change, and keeps the
  layout readable by Dockge. Its audit log is append-only.
- Keel's audit log is insert-only and hash-chained, enforced by the database role, and written in
  the same transaction as the change it records. Inventory revisions are immutable once saved;
  edits create new revisions. Imports MUST NOT overwrite existing objects. Credential values
  stay sealed at rest and MUST NOT appear in logs, events, audit or exports.

**Rationale**: An operator's saved forwards and integrations are their workflow. Losing them on
upgrade costs more trust than any feature earns.

## Supported Platforms & Expected Outcomes

**Tier 1 (expected outcome for the current release line).** Every release MUST install, launch,
connect to a cluster, and pass the feature's quickstart validation on:

| Platform | Architectures | Webview / runtime | Package |
|---|---|---|---|
| Ubuntu 26.04 LTS | x86_64, arm64 | WebKitGTK 4.1 | `.deb`, AppImage |
| Windows 11 | x86_64 | WebView2 (Evergreen) | `.msi` / NSIS installer |
| macOS 26 Tahoe | arm64 (Apple Silicon), x86_64 (Intel) | WKWebView | `.dmg` (universal or per-arch) |

- A Tier 1 regression is release-blocking.
- Binaries MAY be built on older build images (for example, an older glibc for Linux
  compatibility), but validation MUST be done on the Tier 1 OS versions listed above.
- If GitHub Actions has no hosted runner for a Tier 1 OS, the release checklist MUST record a
  manual validation run on that OS before publishing.
- macOS 26 is the last macOS release that supports Intel Macs. When Intel support ends upstream,
  x86_64 macOS moves to Tier 2 through a MINOR amendment.

**Tier 2 (best effort).** This covers older releases such as Ubuntu 22.04/24.04, other
WebKitGTK 4.1 Linux distributions, Windows 10, and macOS 14–15. They are built from the same
artifacts where they run. Bugs are accepted but do not block releases.

**Hosted server (Linux container).** A hosted web build is in scope: `fathom-server` runs as a
Linux container (amd64, arm64) inside a cluster and serves the same UI and the same versioned
protocol to browsers behind OIDC sign-in. It is a shell over the one backend (Principle II), MUST
authenticate to the cluster as the signed-in user, and MUST NOT expose desktop-only features
(kind provisioning, kubeconfig switching, stdio MCP). It does not change the desktop Tier 1 table.

**Depths server (Linux container).** Depths runs as a Linux container (amd64, arm64) on a Docker
host and serves its UI to browsers. Sign-in (OIDC or GitHub) is required; there is no anonymous
mode. Access to the Docker socket is root-equivalent on the host, and the docs MUST say so
plainly. Running behind a Docker socket proxy MUST be supported and is the documented default.
Multi-host agents are out of scope until a later amendment.

**Keel server (Linux container).** Keel runs as Linux containers (amd64, arm64) through compose:
the server, a PostgreSQL database and one or more runners. Sign-in is required; there is no
anonymous mode. Playbooks MUST run only in the separate runner service, which holds no database
credentials and no master key, so a playbook cannot read Keel's stored secrets. The docs MUST say
plainly that a runner has the reach of the credentials granted to the jobs it runs.

**Not supported yet.** Mobile (iOS/Android) is out of scope. The architecture MUST stay
mobile-ready (Principles II and IV), but no mobile outcome is promised.

**Stack.** The backends are Rust (edition 2024) with tokio; Fathom uses kube-rs, and the servers
use Axum and Tower. Keel stores its state in PostgreSQL through sqlx, and its runner image adds
Python and ansible-core, used only as a subprocess there. Depths watches files with `notify`, and telemetry uses an optional
OpenTelemetry exporter (off unless configured). The desktop shell is Tauri v2, and the frontends
are a pnpm workspace of React, TypeScript (strict), Vite, shadcn/ui, and Tailwind, with
CodeMirror 6 for editing, tested with Vitest and Playwright. Replacing a stack component requires an amendment or a plan
that justifies it under Complexity Tracking.

## Development Workflow & Quality Gates

- Features follow the Spec Kit flow: specify → (clarify) → plan → tasks → implement. Every plan
  MUST pass the Constitution Check against the principles above, before research and again after
  design. Any violation MUST be justified in the plan's Complexity Tracking table.
- Every commit MUST be cryptographically signed (GPG or SSH). Unsigned commits and bypassing
  signing are not allowed.
- Commit messages, PR titles and descriptions, and issues MUST NOT contain AI session links.
- Pull requests MUST list which user stories or requirements they satisfy, and MUST pass all
  CI gates (Principle III).
- The old egui UI stays buildable only until the replacement reaches parity. After that, the
  old code, its dependencies, and its tests MUST be removed rather than left to rot.
- Dependencies SHOULD be stable releases. Pre-release crates or packages need a written reason
  in the plan's research.

## Governance

This constitution overrides informal practice and any conflicting guidance in docs, plans, or
agent instructions inside this repository.

**Amendments** are made with `/speckit-constitution`. Each one MUST include a Sync Impact Report,
a version bump, and an update to every affected spec or plan.

**Versioning** follows semantic versioning:
- **MAJOR**: a principle is removed or redefined in an incompatible way, or Tier 1 support for a
  platform is dropped.
- **MINOR**: a principle or section is added, guidance is materially expanded, or a platform is
  moved between tiers.
- **PATCH**: clarifications and wording fixes.

**Compliance**:
- `/speckit-plan` runs the Constitution Check on every plan.
- `/speckit-analyze` flags any conflict between the constitution and a spec, plan, or tasks as
  CRITICAL.
- Reviewers MUST reject changes that violate a MUST without a recorded justification.

**Version**: 1.3.0 | **Ratified**: 2026-09-21 | **Last Amended**: 2026-10-27
