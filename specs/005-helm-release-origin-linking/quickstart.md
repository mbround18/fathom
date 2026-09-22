# Quickstart: Helm Release View & Origin-Context Linking

## Prerequisites

- Rust toolchain + `cargo` (workspace already builds per `docs/MVP.md` setup).
- `pnpm -C ui install` (existing frontend deps — no new ones per research.md R7).
- For live validation: a kubeconfig context with at least one Helm release installed
  (`helm.sh/release.v1` Secrets present in some namespace) — most real clusters qualify. Flux
  origin-linking validation additionally needs a Flux-managed `HelmRelease`/`Kustomization` in that
  cluster (optional — a bare-Helm release with no Flux markers is also a valid, fully-covered case).

## Setup

```bash
cargo build
pnpm -C ui install
```

## User Story 1 — Browse Helm releases (P1)

1. Launch the app against a live or fake-backend context with at least one Helm release.
2. Open the new "Helm Releases" nav entry.
3. **Expect**: a list showing Name, Namespace, Chart, Chart Version, App Version, Revision, Status,
   Last Updated (FR list columns) for every installed release, scoped by the active namespace
   filter (FR-003, research.md R8).
4. Select a release row.
5. **Expect**: Inspector opens (no route change) with Details/Values/Manifest/Revisions/Related
   tabs — values split into user-supplied vs. effective (FR requirement), manifest rendered via the
   existing YAML viewer (003's `YamlEditTab`, read-only here), revision history as a plain list
   (no diff, per spec.md Assumption).

**Independent test**: run against the scripted fake backend with a fixture release list; assert
the list renders all columns and the Inspector opens with all tabs populated, no live cluster
needed.

## User Story 2 — Trace to GitOps origin (P1)

1. Select a Helm release that carries Flux ownership labels
   (`helm.toolkit.fluxcd.io/name`/`namespace`).
2. **Expect**: the Related tab shows a `"reconciled-by"` entry linking to the owning Flux
   `HelmRelease`/`Kustomization`.
3. Click it.
4. **Expect**: Inspector navigates (breadcrumb/back-stack extends, per 004's existing mechanism) to
   a minimal Flux-resource detail view showing Ready/Failed/Reconciling/Suspended/Unknown state,
   the `message` field verbatim, revision, suspended flag, last transition (research.md R4).
5. Click "back" in the breadcrumb.
6. **Expect**: returns to the Helm release detail, exactly where it was.
7. Repeat with a bare-Helm release (no Flux labels).
8. **Expect**: no `"reconciled-by"` entry appears — absence, not an error state (spec.md's explicit
   non-goal framing).

**Independent test**: scripted fake backend with one Flux-owned and one bare-Helm fixture release;
assert the origin link is present/absent respectively, and that clicking it (Flux-owned case)
renders the expected Ready state and pushes/pops the breadcrumb correctly.

## User Story 3 — Inspect full release detail (P2)

1. Select a release with a degraded/undecodable release-storage payload (fixture-only scenario —
   hard to produce on a real cluster).
2. **Expect**: the row still appears, `degraded: true` surfaces in the UI without crashing the
   Inspector, values/manifest render as empty rather than erroring (FR-014, data-model.md).

**Independent test**: unit test `helm::decode_release_payload` against a corrupted fixture blob,
asserting it returns a structured error the caller turns into a degraded summary rather than
propagating a hard failure.

## Automated coverage

- `cargo test` — protocol round-trip tests (contracts/protocol.md), `decode_release_payload` unit
  tests (fixture-encoded blob, no live cluster), `related_for_helm_release` unit tests (owned +
  reconciled-by resolution against synthetic label sets).
- Playwright E2E against the scripted fake backend — one spec per user story above, following the
  existing `ui/tests/e2e/` convention from 002/003/004.

## Manual live verification

```bash
FATHOM_LIVE_CONTEXT=<context-name> pnpm -C ui dev
```

Open "Helm Releases", confirm the list matches `helm list -A` output for the same cluster (compare
independently — do not shell out to `helm` from Fathom itself, per the explicit non-goal); drill
into a Flux-managed release's origin link and cross-check its Ready state against
`kubectl get helmrelease -n <ns> <name> -o yaml`'s `status.conditions`.
