# Phase 0 research — 046 fleet model

Each entry is a question the spec left open, the answer found in the code or the dependency, and
what was rejected. Everything here was verified against this tree, not assumed.

## 1. Is there already a label-selector parser we can use? (FR-004, FR-006)

**Decision**: use `kube::core::{Selector, Expression, SelectorExt}` as the *representation* and
write the string → `Selector` parser ourselves, in the backend.

**Why**: `kube-core-2.0.1/src/labels.rs` gives us exactly three of the four things we need —
`Expression` (`In`, `NotIn`, `Equal`, `NotEqual`, `Exists`, `DoesNotExist`), `Display` for both
`Expression` and `Selector` (so a group's selector round-trips to the string
`ListParams::labels()` takes, which is what hands 047 its scope expression for free), and
`SelectorExt::matches(&BTreeMap<String, String>)` (so matching a member's labels is library code,
not ours). What it does **not** have is a parser: the only conversions in are
`TryFrom<LabelSelector>` / `TryFrom<LabelSelectorRequirement>` from the structured API form, plus
`FromIterator`. There is no `FromStr` and no `parse` anywhere in the file; `ParseExpressionError`
exists only for the `TryFrom` path. So the operator-typed string in FR-004 has no library path in.

Nothing in `src/` uses `Selector` today — every existing label use is a pre-built string
(`ListParams::default().labels(APISERVER_POD_SELECTOR)` at `src/backend/apiserver_auth.rs:1594`,
`…labels(&format!("scaledjob.keda.sh/name={name}"))` at `src/backend/keda.rs:293`). This feature is
the first consumer of the typed form.

**Alternatives considered**:
- *Store the selector as an opaque string and let the API server be the only validator.* Rejected:
  FR-006 requires rejection **at entry** with the parse error verbatim, and a fleet group matches
  local operator-authored labels (Assumptions) — no API server is involved at all.
- *Model groups as `LabelSelector` (matchLabels + matchExpressions) and build them from form
  fields.* Rejected: it is the wizard Principle I rules out, and it cannot express what an operator
  already types into `kubectl -l`.
- *Write our own matcher too.* Rejected: `SelectorExt::matches` is already there and tested
  upstream; re-implementing set semantics is how `in`/`notin` on a missing key goes subtly wrong.

**Consequence for the parser**: it accepts the `kubectl -l` grammar — comma-separated
requirements, `k=v` / `k==v` / `k!=v`, `k in (a,b)` / `k notin (a,b)`, bare `k` (Exists) and `!k`
(DoesNotExist) — and returns the offending substring in the error so FR-006's "verbatim" has
something to be verbatim about. It lives beside the label validator so both error vocabularies are
one module.

## 2. Where does label syntax validation come from? (FR-003)

**Decision**: implement the Kubernetes rules in the backend; there is no validator to borrow.
`k8s-openapi` 0.26.1 ships types, not validation, and `kube` validates nothing client-side.

Rules implemented: a key is an optional DNS-subdomain prefix plus `/`, then a name of 1–63
characters, alphanumeric at both ends, `-`, `_`, `.` inside; a value is empty or the same 1–63
name shape. The failing input and the rule it broke are both in the message (FR-003 verbatim).

**Why the backend and not the UI**: Principle II — one contract. The UI must not hold a second
copy of the grammar that can disagree with the one that persists the file. The UI calls the
command and renders the error it gets back; its own unit tests cover the *decision* logic
(which rows a filter shows), not the grammar.

## 3. Where does fleet metadata live? (FR-007, FR-008)

**Decision**: a new `~/.fathom/fleet.json`, `{ "schema": 1, "members": {...}, "groups": {...} }`,
loaded and saved by new functions in `src/backend/persistence.rs` that follow the `TeamConfig`
precedent at `persistence.rs:669-700` — "unreadable/invalid yields defaults and leaves the file
untouched".

**Why not an existing file**: `src/protocol/v1/team.rs:57-58` says it outright —
`cluster-settings.json` is "§10's field set, exactly — this feature MUST NOT add any other
preference category here (FR-006)". `federation.json` (`persistence.rs:998`) is "context names
only; no secrets" and is owned by 034's runtime. FR-008 is therefore not a preference, it is the
only option the existing files leave.

The `schema: u32` + additive-only rule matches `ClusterSettings { schema: u32, … }`
(`team.rs:61`) and satisfies Principle VI.

## 4. How is sharing done? (FR-012, FR-013, FR-014)

**Decision**: add one file to the existing 014 store — `fleet-definitions.json`, shaped like the
`ProfilesFile` already there — and reuse that module's revision discipline rather than inventing
one.

`src/backend/team_store.rs:1-14` documents the store as a plain operator-controlled directory
holding `connection-profiles.json` (`{ rev, profiles }`) and `cluster-settings/<context>.json`,
with every write optimistic-concurrency-checked against the `rev` the caller last saw
(`StoreError::Conflict`) and written atomically via temp-file + rename. `read_profiles`,
`publish_profile`, `remove_profile`, `write_profiles_file` are the shapes to mirror.

Origin and local-wins already have a precedent to copy rather than design:
`ClusterSettingsFieldOrigin` with a `SettingOrigin` per field (`team.rs:44`), and
`ConnectionProfile.removed` (`team.rs:137`) — "kept (not deleted) so an operator who already
adopted it can be told 'no longer shared' instead of losing the row". FR-014 is that comment.

Command surface mirrors the existing one in `src/backend/cmd/team.rs:30-78`:
`RefreshConnectionProfiles` / `PublishConnectionProfile` / `AdoptConnectionProfile` /
`RemoveConnectionProfile`, including the `gate(state, sender, FathomRole::Editor)` role check on
publish.

**Alternative rejected**: a second store, or a per-member file. Both break the spec's own
Assumption that 014's store is *the* sharing mechanism, and the per-file layout loses the single
`rev` that makes a conflict detectable.

## 5. Where does the protocol live and how do the TS bindings stay honest? (FR-019)

**Decision**: extend `src/protocol/v1/fleet.rs` (136 lines today, holding
`FleetContextSummary`), add commands to the `BackendCmdV1` enum in `src/protocol/v1/cmd.rs`
(2017 lines, `#[serde(tag = "type", content = "data")]`), add events beside
`FleetSummaryUpdated` at `src/protocol/v1/event.rs:538`, and add the hosted policy arms in
`src/protocol/v1/mutation.rs`.

Bindings: `tests/export_bindings.rs` regenerates `ui/src/protocol/generated/` from
`BackendCmdV1::export_all` + `BackendEventEnvelopeV1::export_all`; `scripts/ci.sh:75-83` snapshots
that directory, re-runs the test and `diff -r`s, failing with "ui/src/protocol/generated is
stale". So a new command type needs no registration step — it needs `cargo test -p fathom --test
export_bindings` run and the output committed, or the gate fails. Round-trip tests go in the
existing `#[cfg(test)]` blocks at the tail of `cmd.rs` (`roundtrip(BackendCmdV1::…)`, e.g. line
1961) and `event.rs`.

Dispatch: a new `src/backend/cmd/fleet.rs` handler in the `handle(cmd, state, sender) ->
Option<BackendCmd>` chain that `cmd/team.rs` uses — each family claims its commands and returns
the rest.

## 6. Where does the fleet UI live? (FR-002, FR-016, FR-017)

**Decision**: in place. `FleetDashboardView` is mounted from
`ui/src/components/shell/TopBar.tsx:121` as a sheet; it maps `session.contexts` straight to
`FleetContextCard` in kubeconfig order (`FleetDashboardView.tsx:60`). Grouping, filtering and the
coverage line go into that view; the display name goes into the card beside the context string at
`FleetContextCard.tsx:57` (both shown — FR-002), not instead of it.

The mount-scoped `start()`/`stop()` fleet polling already in that view is the Principle V
precedent; FR-015 adds nothing to it.

## 7. What does the test surface look like? (Principle III)

**Decision**: three layers, all of which already exist.
- **E2E**: Playwright against the scripted fake backend — `ui/tests/e2e/*.spec.ts` with
  `import { emit, open, sent } from './helpers'` and `batch`/`system` builders from
  `../fixtures/builders` (see `ui/tests/e2e/dock-and-nav.spec.ts:13-18`). One spec per user story.
- **Unit (vitest)**: the UI's decision logic — which members a filter or group shows, and the
  "showing n of m" count. Not the selector grammar, which is the backend's.
- **Rust**: serialization round-trips for every new command/event variant, plus parser and
  validator tables, plus persistence fallback (corrupt file → defaults, file unmodified).

Playwright worker sizing is automatic (`ui/tests/support/workers.ts`); no `--workers` flag.
