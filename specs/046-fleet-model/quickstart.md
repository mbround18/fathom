# Quickstart — validating 046 fleet model

Runnable checks that prove the feature works end to end. Details of types live in
`data-model.md`, of commands in `contracts/protocol.md`; this file is how you run it.

## Prerequisites

- Rust toolchain (edition 2024), `pnpm` — run every `pnpm` command from `ui/`, not the repo root.
- `CARGO_BUILD_JOBS=4` when other agents are active on this machine (CLAUDE.md).
- `fathom-dev` kind cluster for the live scenario only. The E2E scenarios need no cluster: they
  run against the scripted fake backend.
- Playwright sizes its own workers (`ui/tests/support/workers.ts`); pass no `--workers`.

## 1. Protocol and bindings are honest

```bash
cargo test -p fathom --test export_bindings      # regenerates ui/src/protocol/generated/
git diff --stat ui/src/protocol/generated          # must be committed, not left dirty
cargo test -p fathom fleet                         # round-trips, parser, validator, persistence
```

Expected: every new command and event variant has a passing round-trip; `ui/src/protocol/generated`
is committed. `scripts/ci.sh` fails the build on drift, so an uncommitted diff here is a red gate.

## 2. Selector and label grammar

```bash
cargo test -p fathom fleet_selector
cargo test -p fathom fleet_labels
```

Expected: `env=prod`, `env!=prod`, `env in (prod,staging)`, `tier notin (edge)`, `managed`,
`!managed` and comma-joined combinations all parse; each round-trips through `Display` back to a
string `ListParams::labels()` accepts. Malformed input (`env in prod`, `=prod`, `env=`, a 64-
character label name, a key with two slashes) is rejected, and the error names the offending
substring verbatim — that verbatim string is what FR-006 and FR-003 promise the operator.

## 3. Persistence survives a corrupt file (SC-003, Principle VI)

```bash
cargo test -p fathom fleet_persistence
```

Expected: a truncated or non-JSON `fleet.json` yields in-memory defaults, emits
`FleetModelLoadFailed` with the verbatim reason, and leaves the file **byte-for-byte unchanged** —
assert on the file's bytes before and after, not just on the absence of an error. A file written at
`schema: 1` with an unknown extra key still loads.

## 4. User story 1 — name and label a member

```bash
cd ui && pnpm test:e2e fleet-model-identity
```

Expected: a context with an ARN-shaped name shows its display name and its context string on the
same card, the context string copies verbatim, an invalid label is rejected with the backend's own
message and nothing is persisted, and both values survive a reload. This is SC-001.

## 5. User story 2 — group by selector and roll up

```bash
cd ui && pnpm test:e2e fleet-model-groups
```

Expected: one selector expression defines a group and its members appear with no per-cluster filing
step; labelling a new cluster joins it to every matching group with no further action (SC-002); a
group matching nothing says so and is not removed; ungrouped members are reachable; and a rollup
over a group where one member is denied shows `n of m` with that member **named** and labelled
*not permitted* — distinct from absent and from unreachable. That last assertion is SC-004 and the
one this feature exists to get right.

## 6. User story 3 — share through the 014 store

```bash
cd ui && pnpm test:e2e fleet-model-sharing
cargo test -p fathom fleet_store
```

Expected: publishing then adopting reproduces the same group names and memberships for a second
operator (SC-006); a locally overridden label wins, is marked local, and the shared value it
overrode stays visible; and when the store goes unreachable or the definition is removed upstream,
the adopted copy is **kept** and marked no longer following — never deleted (FR-014).

## 7. Hosted stays a fleet of one (FR-018)

```bash
cd ui && pnpm test:hosted fleet
cargo test -p fathom hosted_fleet
```

Expected: display name, labels and sharing work for `home`; a command naming any other context is
denied; and `StartFleetPolling`, `StopFleetPolling`, `RequestFederationSnapshot` and a
non-`home` `SetContext` all still deny exactly as before.

## 8. Quiet and fast (SC-005, SC-007, Principle V)

```bash
cd ui && pnpm test:unit fleet-filter      # filter/sort decision logic, 100-member fixture
./scripts/measure-idle-cpu.sh             # unchanged against the pre-046 figure
```

Expected: filtering a 100-member fleet responds under 100 ms, and idle CPU is indistinguishable
from before the feature — nothing here polls, watches or holds a client.

## 9. Full gate

```bash
./scripts/ci.sh
./scripts/hygiene.sh
```

Expected: `ci.sh` green, including the bindings-drift step; `hygiene.sh` exits 0 with nothing
dangling.
