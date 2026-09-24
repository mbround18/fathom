# Data Model: Cluster Sanitizer / Linter

All types live in `ui/src/linter/types.ts`. Nothing is persisted (FR-013); every value is
recomputed from `store.lists` / `store.details`.

## Enumerations

- `Severity`: `'high' | 'medium' | 'low'` (fixed per rule, spec Assumptions).
- `Category`: `'resource-limits' | 'probes' | 'api-deprecation' | 'unused-object' | 'naked-pod' | 'rbac' | 'pdb'`.
- `RuleId`: `'resource-limits' | 'probes' | 'deprecated-api' | 'unused-config' | 'unused-serviceaccount' | 'naked-pod' | 'rbac-wildcard' | 'missing-pdb'`.

## Finding

| Field | Type | Notes |
|---|---|---|
| `id` | string | `${rule}|${kind}|${namespace ?? '*'}|${name}|${discriminator}` (stable across recomputes) |
| `rule` | RuleId | producing rule |
| `category` | Category | |
| `severity` | Severity | |
| `message` | string | human-readable, names the container/verb/etc. |
| `ref` | `ObjectRef` | `{ section, kind, namespace, name }`, passed unchanged to `openInspector` |

## RuleResult

`{ rule, category, status: 'evaluated' | 'pending' | 'skipped', reason?: string, checked: number, flagged: number }`

- `skipped`: a required list section is not `ready` in the cache (RBAC/visibility, never fetched),
  or the rule is structurally unavailable (ServiceAccounts, research R3). Never yields findings (FR-010).
- `pending`: required per-object details are still being fetched; partial findings are still shown.
- `checked`: number of objects the rule examined; a rule with `checked === 0` is excluded from the score.

## LintReport

`{ findings: Finding[], rules: RuleResult[], counts: Record<Severity, number>, score: number | null, pendingObjects: number }`

- `score` = round(100 * (sum(checked) - sum(flagged)) / sum(checked)) over non-skipped rules with
  `checked > 0`; `null` when nothing was checked.
- `pendingObjects`: objects whose detail fetch is queued/in-flight (drives the "still evaluating N" banner).

## LintInput (pure boundary of `evaluate`)

```ts
interface LintInput {
  rows(section: string): { status: 'idle'|'loading'|'ready'|'error'; rows: readonly unknown[] } | undefined
  detail(kind: string, namespace: string | null, name: string): { data: unknown } | undefined
}
```

Decoupled from Zustand exactly like `topology/build.ts`'s `BuildInput`.

## Data sources per rule

| Rule | Lists | Details (`raw_json` parsed) |
|---|---|---|
| resource-limits | deployments, statefulsets, daemonsets, pods | workload spec.template containers; naked Pod spec |
| probes | deployments, statefulsets, daemonsets | workload spec.template containers |
| deprecated-api | (all below) | `metadata.managedFields[].apiVersion` of any loaded detail |
| unused-config | configmaps, secrets, pods, workloads | pod + workload env/envFrom/volumes/projected/imagePullSecrets |
| unused-serviceaccount | serviceaccounts | always skipped (no backend capability) |
| naked-pod | pods | Pod `metadata.ownerReferences` |
| rbac-wildcard | roles, clusterroles | `RoleDetails.rules` / `ClusterRoleDetails.rules` |
| missing-pdb | deployments, statefulsets, replicasets, poddisruptionbudgets | workload template labels + ownerReferences; PDB `spec.selector` |
