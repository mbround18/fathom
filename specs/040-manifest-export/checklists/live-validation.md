# Live validation: 040 manifest export

**Date**: 2026-09-26
**Context**: `kind-fathom-dev` only. The production `default` context was never used: the spec's
dry-run helper refuses `default`, and the devbridge is pinned to its own context.
**How**: `FATHOM_LIVE_CONTEXT=kind-fathom-dev pnpm -C ui exec playwright test -c
playwright.live.config.ts tests/live/manifest-export.spec.ts --project=chromium-wide`
(`ui/tests/live/manifest-export.spec.ts`). **Result**: 3 passed (10.1s).

## Checks

- [x] **Clean single export re-applies (SC-001)**. The `fathom-dev/web` Deployment was downloaded
  from the Inspector YAML tab with "clean for re-apply" on, as `deployment-fathom-dev-web.yaml`.
  The file has no `resourceVersion`, `uid`, `managedFields`, `creationTimestamp` or `status`.
  `kubectl --context kind-fathom-dev apply --dry-run=server -f <file>` reported
  `deployment.apps/web configured|unchanged (server dry run)`.
- [x] **Clean bundle re-applies (SC-002)**. `web` and `scale-me` were selected and exported
  with the selection bar's Export YAML as one file of two documents. The server dry-run apply
  accepted both objects.
- [x] **Kubeconfig export (SC-004/SC-005)**. The `kind-fathom-dev` context was exported to a
  temporary file:
  - the preview showed the context with the embedded client key flagged and redacted (no PEM
    in the page)
  - credentials were included only after ticking the opt-in box, and the write happened only
    after the confirm step
  - the file has mode `0600`
  - `kubectl --kubeconfig <file> get namespace fathom-dev` succeeded
  - the bridge logged only `context`, `path` and `embedded=true`, never credential material
  - the temporary directory was deleted afterwards
- [x] Nothing was persisted to the cluster: every apply was `--dry-run=server`.
