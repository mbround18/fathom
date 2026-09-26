# Live validation: 026 Mutation audit-log viewer

- **Date**: 2026-09-26
- **Cluster**: `kind-fathom-dev`, namespace `fathom-dev`, through the devbridge (real backend, real
  `~/.fathom/audit-log.json`)
- **How**: `ui/tests/live/qs-026-audit-log.spec.ts` (`chromium-wide`)

| Step | Result | Notes |
|---|---|---|
| US1 a mutation appears at the top with its action, target, context, outcome and time | PASS | **Deviation:** scaled `scale-me` 1→2→1 instead of cordoning a node, because the devbridge refuses cluster-scoped writes |
| US1 a failed action is recorded as Failed with the reason | PASS | the tap rewrites a pod delete to a missing name, so the API server answers 404 |
| US2 filtering by action type narrows the list | PASS | |
| US2 filtering by outcome "Failed" narrows the list | PASS | |
| US2 the name search narrows case-insensitively | PASS | |
| US2 clearing the filters restores the full list | PASS | |
| US3 retention bound (500) | NOT RUN LIVE | unit tests only; 500 live mutations were not driven |
| US3.3 no cluster-side (API-server) audit data is requested | PASS | no `*ClusterAudit*` command sent |

Automated gate (T035), same day:
- `pnpm -C ui test`: 1283/1283.
- Full Playwright suite on all 14 projects (includes `audit-log.spec.ts`): 4743 passed, 380 skipped. The 15 failures under parallel load all pass serially, apart from 2 WebKit clipboard harness failures, which were fixed.
