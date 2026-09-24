# Contract: `ui/src/linter/` module

Pure, React-free, never throws, never dispatches commands (mirrors `contracts/topology-graph-module.md`).

```ts
// evaluate.ts
export function evaluate(input: LintInput): LintReport

// each rules/*.ts
export interface Rule { id: RuleId; category: Category; severity: Severity
  run(input: LintInput): RuleOutcome }   // RuleOutcome = RuleResult & { findings: Finding[] }

// fetchPlan.ts (pure; the panel performs the sends)
export function detailTargets(input: LintInput): DetailTarget[]  // {kind, namespace, name}
export function nextBatch(targets, arrived: Set<string>, inflight: Map<string, number>, limit: number, now: number): DetailTarget[]
```

Guarantees:

1. A rule whose required list section is not `ready` returns `status: 'skipped'` with a reason and no findings.
2. `unused-serviceaccount` always returns `skipped`.
3. Malformed/absent `raw_json` on one object never aborts the pass; that object is treated as not yet evaluated (pending).
4. Findings are attributed to the owning controller, not its generated ReplicaSet/Pod (spec Edge Cases).
5. No command other than the read-only `Refresh*`/`Select*` commands (via `refreshSection` / `inspect`) is ever sent by the Linter; no mutating command is reachable from `LinterPanel` (FR-003, SC-005).
6. Detail fetches: bounded in-flight limit (8), active namespace scope only, entries expire after 10 s so one lost reply cannot stall the queue.
