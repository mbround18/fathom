# Implementation Plan: Keel host provisioning and convergence

**Branch**: `068-keel-converge` | **Date**: 2026-11-17 | **Spec**: [spec.md](spec.md)

## Summary

A profile in `.keel/jobs.yaml` gains a recipe (`includes`, then `jobs`). Keel expands a host's
profiles into an ordered plan and runs it as a **converge**: one parent run, approved once, with
one child run per distinct job so logs, results, applied revisions and signed-mode review keep
working per step. A `holding` row per converge host, guarded by a partial unique index, keeps
other runs off the host for the whole chain; a database trigger advances or skips the chain on
every terminal step outcome. On top of that: converge on enrollment, admin profile edits with
group-derived suggestions, `covers` so narrow path jobs fold into a converge, check mode, per-job
environment credentials through the `SecretResolver` port, a forge commit-status gate, and a
`choice` param kind.

Research: [research.md](research.md). It corrects three spec assumptions (R1a gameserver already
imports prox-vm; R9 the forge port cannot read statuses; R8 `secrets()` bypasses the port) and
finds two rollout hazards (R4 old runners refuse new manifest keys; R5 no ceiling allows
`debian`).

## Technical Context

**Language/Version**: Rust (edition 2024, `rust-toolchain.toml`) for keel-core, keel-protocol,
keel-exec, keel-trust, keel-forge, keel-server, keel-runner. TypeScript, React and Vite for
`apps/keel/web`. Markdown for `docs/guides/keel`.

**Primary Dependencies**: existing only. sqlx, axum, tokio, serde, serde_yaml, ts-rs, russh,
reqwest (forge adapters), opentelemetry. No new crates; `proptest` is already a dev-dependency
(067 T064) and is reused for the expansion property tests.

**Storage**: PostgreSQL migration `0018_converge.sql` ([data-model.md](data-model.md)). 0015 and
0017 are reserved by 067 (R12).

**Testing**:

- `cargo test -p keel-core converge` and `cargo test -p keel-core manifest`: recipe expansion,
  cycles, coverage closure, effective plan, `choice`, `env`, `groups` — pure, no I/O.
- `cargo test -p keel-trust ledger`: `Checked` outcome.
- `cargo test -p keel-forge statuses` against recorded fixtures.
- `cargo test -p keel-runner`: `check`, `env_var` items, `choice` re-check, `setsid`/prompt.
- `cargo test -p keel-server --test converge`, `--test converge_lock`, `--test coverage`,
  `--test host_profiles`, `--test check_mode`, `--test env_canary`, `--test required_checks`,
  `--test choice_params`. Postgres-backed; they fail under `CI=1` without
  `KEEL_TEST_DATABASE_URL` (067 D4).
- Vitest for plan/step derivations; Playwright project `s068`; `polish-screens` last.

**Target Platform**: Keel server container (amd64, arm64); `keel-runner` on Debian with systemd.

**Project Type**: web service + host agent + web UI.

**Performance Goals**: planning 100 hosts × 10 steps under 200 ms server side (pure expansion is
microseconds; the cost is the inserts, done in one statement per table). The advance trigger adds
one indexed update per terminal step. Dispatch's extra `not exists` uses
`converge_hosts_one_holding`.

**Constraints**:

- A manifest without new keys behaves exactly as before (FR-016, SC-006).
- Converge never widens a host's ceiling (FR-017, R5).
- No credential value outside the sealed store, logs or audit (SC-005).
- Old runners fail closed on every new wire field (R4).

**Scale/Scope**: tens of hosts per workspace; ≤ 32 steps per host plan (validated).

## Constitution Check

*Pre-research and post-design: both pass. One entry in Complexity Tracking (the trigger).*

| Principle | How this plan complies |
|---|---|
| I. Explicit mutation | Converge, retry, profile edits, suggestion acceptance, required checks and check runs all go Preview → Confirm. Profile edits need step-up. The converge preview lists every host × step, the commit, inventory revision, params and credential **names**. Auto-converge on enrollment is visible as a checkbox and audited as `system:enroll`. |
| II. One contract | All new commands, events and types are in `keel-protocol` with ts-rs bindings (contracts/protocol.md). Recipe expansion, coverage and effective plans are computed once in keel-core and used by both server (planning) and runner (validation). |
| III. Tested at boundaries | Pure core functions get unit and property tests first. Every new command/event gets a round-trip test. Each story has an integration test that fails without it, and Playwright `s068` against the fake backend. SC-004 and SC-005 are explicit tests. |
| IV. Density | The converge page is a host × step matrix that collapses to a per-host list under 640 px. Hosts gains a compact "converge" column and a suggestion dot. Works at 360 px and by keyboard. |
| V. Fast and quiet | Step state changes ride the existing run events; no new polling in the web. The commit-status poller runs on the sync loop's cadence, only while a run waits. |
| VI. User data | Audit in the same transaction as every change. Credentials resolve at dispatch only; `env:` values never reach a run record. Profile removals revoke principals by re-issuing the cert. |
| Keel server rules | The server still executes nothing. Commit statuses are read from the forge, never computed. |
| Keel host runner rules | The runner re-validates every step: manifest, ceiling, params (incl. `choice` against its checkout), env names. Signed mode, ledger and approvals keep their meaning; `Checked` is a non-moving outcome. |
| Stack | Rust, tokio, Axum, sqlx, React, Vite. Nothing replaced or added. |
| Global rules | **Agnostic core + adapters**: `keel-core::converge` (pure: expand, plan, coverage, effective profiles, suggestions) with no I/O; server, queue and runner are adapters. Forge statuses are a new `Forge` port method with Forgejo/GitHub adapters. Env values go through the `SecretResolver` port. **OTel**: spans `converge.plan` (hosts, steps), `converge.step` (job, outcome), `profile.suggest` (hosts, adds, removes), `checks.wait` (contexts, outcome). **Flag**: runtime `KEEL_CONVERGE` (default off until US1–US3 land) gates the converge commands, auto-converge and the UI entry points; with it off, recipes still validate but nothing plans. |

## Architecture: agnostic core + adapters

### Core (`crates/keel-core/src/converge.rs`, new; pure)

```rust
pub struct Recipe { pub includes: Vec<String>, pub jobs: Vec<String> }

/// Depth-first: includes (in order, recursively), then jobs. First position wins.
pub fn expand(profiles: &[String], m: &Manifest) -> Result<Vec<String>, ExpandError>;
/// ExpandError::Cycle(Vec<String>) names the cycle; ::UnknownProfile; ::TooManySteps.

/// Transitive closure of `covers`; refuses cycles and self-cover.
pub fn coverage(m: &Manifest) -> Result<BTreeMap<String, BTreeSet<String>>, CoverError>;

/// One host's plan: `expand(assigned)`, then jobs covered by another step of the same plan are
/// dropped (k3s-master + k3s-audit ⇒ k3s-master only). Steps come from the recipes, not from
/// each job's `profiles:` list (that list still decides who a direct run targets). The host's
/// ceiling, checked by the runner per step, is the guard (FR-017, research R5).
pub fn host_plan(assigned: &[String], m: &Manifest, cov: &Coverage) -> Vec<PlannedStep>;

/// Does a pending converge with these remaining jobs cover `job`?
pub fn covers(remaining: &[String], job: &str, cov: &Coverage) -> bool;

/// groups: map ⇒ (adds, removes) for one host, given its inventory groups and assigned profiles.
/// Removes are only suggested for profiles some group mapping could have produced.
pub fn suggest(groups: &[String], assigned: &[String], m: &Manifest) -> Suggestion;

/// Strictest policy across steps: any requires_approval, max approvals_needed, any four_eyes,
/// min ttl.
pub fn combine_policies(steps: &[ApprovalPolicy]) -> ApprovalPolicy;
```

`manifest::validate` calls `expand` for every profile (cycles refused at sync and on the runner),
`coverage`, and checks `env` names with the same refusal list the executor uses (moved to
`keel-core::env_names` so both share it), `choice` shapes and `groups` targets.

### Adapters

- **Server, planning** (`apps/keel/server/src/converge.rs`, new): `preview_converge`,
  `create_converge`, `retry_converge`, `cancel_converge`. Resolves hosts (target selection ∩
  not revoked), pins commit (`Mirror::pin`) and inventory, calls `host_plan` per host, computes
  the combined policy, runs `review::early_check` per child run in signed mode, inserts parent
  run, child runs, `converge_hosts`, `converge_steps`, `run_hosts` (first step `blocked`, others
  `blocked`) in one transaction, audits `converge_created`.
- **Server, queue** (`queue.rs`): dispatch filter and `waiting → holding` on claim; the
  `converge_advance` trigger (migration) does the rest; `recover_in_flight` reconcile; `finished`
  skips revision writes for check runs; `secrets()` through `SecretResolver` plus `env_var` items;
  `runner_too_old` refusal by capability.
- **Server, approvals** (`approvals.rs`): approving a `converge` parent moves every host's first
  step `blocked → queued`; deny/expire/cancel cascades to children (`cancelled`, hold released).
- **Server, coverage** (`runs.rs::create_run`): for path-trigger runs, `covered` per host.
- **Server, enrollment** (`runner_listener.rs::enrol` → `converge::on_enrolled`).
- **Server, profiles** (`hosts.rs`): `PreviewHostProfiles`/`ConfirmHostProfiles`, re-issue cert,
  `profile_suggestions` refresh in `sync.rs`.
- **Server, checks** (`checks.rs`, new): required checks on bindings; poller.
- **Forge** (`crates/keel-forge`): `commit_statuses` for Forgejo and GitHub.
- **Trust** (`crates/keel-trust`): `Outcome::Checked`.
- **Exec** (`crates/keel-exec/src/frames.rs`): `Assign.check`, `SecretItem` kind `env_var`.
- **Runner** (`executor/`): `--check --diff`, env injection, `choice` re-check, `setsid`.
- **Web** (`apps/keel/web/src/features/`): `converge/` (new: preview, matrix page, retry),
  `hosts/` (profile editor, suggestions, enrollment checkbox, converge column), `runs/` (check
  label, covered state, waiting_checks), `bindings/` (required checks), Start dialog params.

## Dependencies (outside this spec)

| # | Gap | Gate |
|---|---|---|
| D1 | 067 D2 (per-host run lock, 0016) | done; this branch builds on it |
| D2 | 067 D3 join tokens (0015) | blocks the join-acceptance half of FR-006 only (T046) |
| D3 | 067 `Hello.capabilities` | needed for `runner_too_old` (T012); if 067 has not merged, 068 adds the field itself with the same shape |
| D4 | Runner upgrade across the fleet | must precede the ansible delta landing (R4) |
| D5 | ../ansible delta (contracts/ansible.md) | handed to the ansible session; SC-001/SC-002 verified after it lands |

## Phasing (story order for tasks.md)

1. **Setup + Foundational**: migration 0018, core `converge` module and manifest keys, protocol
   types, frame fields, Playwright `s068`, flag.
2. **US1** (P1, MVP): plan, preview, approve once, chain with lock and advance, failure skip,
   retry, converge page.
3. **US2** (P1): auto-converge on enrollment; join acceptance after D3.
4. **US3** (P1): profile editor with step-up, cert re-issue, `groups:` suggestions.
5. **US4** (P2): `covers`, `covered` outcome, narrow jobs.
6. **US5** (P2): check mode end to end incl. ledger `Checked`.
7. **US6** (P2): `env:` through `SecretResolver`, canary test.
8. **US7** (P3): forge statuses, required checks, `waiting_checks`.
9. **US8** (P3): `choice` params, Start dialog params, `setsid`.
10. **Polish**: docs, ansible contract handoff, impeccable pass, brochure screens last.

## Project Structure

### Documentation (this feature)

```text
specs/068-keel-converge/
├── spec.md
├── plan.md
├── research.md
├── data-model.md
├── quickstart.md
├── contracts/
│   ├── manifest.md    # new .keel/jobs.yaml keys and validation
│   ├── protocol.md    # keel-protocol and keel-exec frame additions
│   ├── ansible.md     # the ../ansible delta for the ansible session
│   └── ui.md
└── tasks.md
```

### Source Code (repository root)

```text
crates/keel-core/src/converge.rs              (new: expand, coverage, host_plan, suggest, combine_policies)
crates/keel-core/src/env_names.rs             (new: shared refusal list)
crates/keel-core/src/{manifest,policy,ports,lib}.rs
crates/keel-protocol/src/converge.rs          (new) + {cmd,event,run,host,binding,lib}.rs
crates/keel-exec/src/frames.rs                (Assign.check, SecretItem env_var)
crates/keel-trust/src/ledger.rs               (Outcome::Checked)
crates/keel-forge/src/{forgejo,github}.rs     (commit_statuses)
apps/keel/server/migrations/0018_converge.sql
apps/keel/server/src/converge.rs              (new)
apps/keel/server/src/checks.rs                (new)
apps/keel/server/src/{queue,runs,approvals,hosts,runner_listener,sync,api,config,review}.rs
apps/keel/server/tests/{converge,converge_lock,coverage,host_profiles,check_mode,env_canary,required_checks,choice_params}.rs
apps/keel/runner/src/executor/{mod,process,secrets}.rs
apps/keel/runner/tests/{check_mode,env_vars,choice,prompt}.rs
apps/keel/web/src/features/converge/          (new)
apps/keel/web/src/features/{hosts,runs,bindings,settings}/
apps/keel/web/tests/e2e/s068/, tests/support/fake-converge.ts
docs/guides/keel/converge.md
```

**Structure decision**: all planning logic is pure and lives in keel-core so the runner validates
the same recipes the server plans. The server owns persistence and sequencing; the database owns
the lock invariant (unique index) and the chain advance (trigger).

## Complexity Tracking

| Violation | Why needed | Simpler alternative rejected because |
|---|---|---|
| A PL/pgSQL trigger (`converge_advance`), the first trigger in Keel's schema | Step outcomes are written from five code paths (`finished`, `disconnected`, `recover_in_flight`, `fail_host`, `cancel_hosts`). A missed path leaves a host holding forever. | Calling `converge::advance` from each path works today but a sixth writer can forget it. Kept as owner question Q5; switching is local to tasks T020 and T021. |

## Open questions for the owner

| # | Question | Recommendation | Where it matters |
|---|---|---|---|
| Q1 | A recipe step's job (e.g. `debian` under `keel-node`) is refused by every current ceiling. Add `debian` to the ansible ceilings, or have Keel derive "effective profiles" from recipes? | Ceilings (the host stays the authority; FR-017). | research R5, contracts/ansible.md §5, T045/T068 |
| Q2 | Signed-mode override/baseline assertions bind one run id and job. Keep one assertion per step (presented together), or change keel-trust to sign a plan digest? | Per step for 068; plan digest as a follow-up spec. | research R3, T018 |
| Q3 | Does Retry of a converge go through approval again? | Yes, same policy (a retry runs again on the host). | T023 |
| Q4 | Keep manifest `version: 1` with a runner floor, or bump to `version: 2` so old runners fail with a clear message? | Keep 1 + floor (`runner_too_old`, binding banner). | research R4, T015 |
| Q5 | Advance the chain with a Postgres trigger (first in Keel) or a Rust call from the five writer paths? | Trigger. | Complexity Tracking, T020 |
