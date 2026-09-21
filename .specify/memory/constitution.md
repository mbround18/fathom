# Fathom Constitution

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

### II. One Contract, Backend Owns the Cluster

- All Kubernetes access, persistence, and long-running work (watches, log streams, port forwards,
  metrics, the MCP server) MUST live in the Rust backend.
- The UI MUST talk to the backend only through the versioned command/event protocol in
  `src/protocol/`. UI code MUST NOT call the backend's internals or read or write
  `~/.fathom/` directly.
- The protocol MUST be defined once, in Rust. Any other client (the TypeScript UI, or future mobile
  shells) MUST use types generated from it. A breaking change on one side MUST fail the build
  or tests, not show up at runtime.
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

**Not supported yet.** Mobile (iOS/Android) and a browser-hosted web build are out of scope. The
architecture MUST stay mobile-ready (Principles II and IV), but no mobile outcome is promised.

**Stack.** The backend is Rust (edition 2024) with tokio and kube-rs. The desktop shell is
Tauri v2, and the frontend is pnpm, React, TypeScript (strict), Vite, shadcn/ui, and Tailwind,
tested with Vitest and Playwright. Replacing a stack component requires an amendment or a plan
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

**Version**: 1.0.0 | **Ratified**: 2026-09-21 | **Last Amended**: 2026-09-21
