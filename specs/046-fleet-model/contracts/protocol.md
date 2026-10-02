# Interface contract — 046 fleet model

Fathom's external interface to its own UI is the command/event protocol. It is defined once in
Rust and the TypeScript is generated (Principle II), so this document names the variants and their
obligations; the field types are in `data-model.md` and the authority is the Rust.

- Commands: `BackendCmdV1` in `src/protocol/v1/cmd.rs`, adjacently tagged
  (`{ "type": "…", "data": { … } }`). Fire-and-forget; results arrive as events.
- Events: `BackendEventV1` in `src/protocol/v1/event.rs`.
- Types: `src/protocol/v1/fleet.rs`.
- Hosted policy: one arm per new command in `src/protocol/v1/mutation.rs`.
- Bindings: `cargo test -p fathom --test export_bindings` regenerates
  `ui/src/protocol/generated/`; `scripts/ci.sh` fails the build if the committed output drifts.
- Every variant below needs a serialization round-trip test (FR-019, Principle III).

## Commands

| Command | Data | Obligation |
|---|---|---|
| `RefreshFleetModel` | — | Load `~/.fathom/fleet.json`, reconcile member availability against the current kubeconfig, emit `FleetModelUpdated`. Starts no background work (FR-015). |
| `SetFleetMemberDisplayName` | `{ context, display_name: Option<String> }` | Trim; empty → `None`. Persist, mark the field origin local, emit `FleetModelUpdated`. |
| `SetFleetMemberLabels` | `{ context, labels: BTreeMap<String, String> }` | Validate every key and value before writing anything. On failure emit `FleetModelRejected` with the verbatim reason and persist nothing (FR-003). |
| `ForgetFleetMember` | `{ context }` | The only path that removes a member's metadata; permitted whether the member is `Present` or `Absent` (FR-009). |
| `SaveFleetGroup` | `{ name, selector }` | Parse the selector first. On a parse error emit `FleetModelRejected` carrying the error verbatim and create no group (FR-006). Upsert by name. |
| `DeleteFleetGroup` | `{ name }` | Remove the group. Members are untouched. |
| `RequestFleetGroupRollup` | `{ group }` | Aggregate the summaries already held for the matching members and emit `FleetGroupRollupUpdated`. Issues no new cluster requests of its own (FR-015). |
| `RefreshSharedFleetDefinitions` | — | Read `fleet-definitions.json` from the 014 store; emit `SharedFleetDefinitionsUpdated`, or `FleetStoreUnavailable` with the store's own reason. |
| `PublishFleetDefinition` | `{ id }` | Requires `FathomRole::Editor`, as `PublishConnectionProfile` does. Optimistic-concurrency-checked against the last seen `rev`; a conflict is reported, never overwritten. |
| `AdoptFleetDefinition` | `{ id }` | Copy locally. Must not overwrite any value whose origin is local (FR-013). |
| `RemoveFleetDefinition` | `{ id }` | Marks the store row `removed`. Adopted local copies survive (FR-014). |

## Events

| Event | Data | Obligation |
|---|---|---|
| `FleetModelUpdated` | `{ members: Vec<FleetMember>, groups: Vec<FleetGroup> }` | The whole model. Group membership is computed here, not stored. |
| `FleetModelRejected` | `{ reason: String, field: Option<String> }` | The verbatim validator or parser error. Emitted instead of, never alongside, a successful update. |
| `FleetModelLoadFailed` | `{ path, reason }` | The file could not be read or parsed. Defaults are in use and the file was left untouched (FR-007). |
| `FleetGroupRollupUpdated` | `FleetGroupRollup` | Always carries `members_covered`, `members_total` and `gaps` (FR-010). |
| `SharedFleetDefinitionsUpdated` | `{ definitions: Vec<SharedFleetDefinition>, adopted: Vec<AdoptedFleetDefinition> }` | Origins resolved; `still_shared: false` rows included, not dropped (FR-014). |
| `FleetStoreUnavailable` | `{ reason: String }` | `StoreError`'s own message. Adopted copies remain usable. |

## Hosted policy (FR-018)

A hosted install is a fleet of exactly one member — the `home` context. Display name, labels and
shared-store participation all work there.

| Command | Hosted |
|---|---|
| `RefreshFleetModel`, `SetFleetMemberDisplayName`, `SetFleetMemberLabels`, `SaveFleetGroup`, `DeleteFleetGroup`, `ForgetFleetMember` | Allowed, scoped to `home`. A command naming any other context is denied. |
| `RequestFleetGroupRollup` | Allowed; a group over one member is a rollup of one, with coverage stated as `1/1`. |
| `RefreshSharedFleetDefinitions`, `PublishFleetDefinition`, `AdoptFleetDefinition`, `RemoveFleetDefinition` | Allowed under the signed-in user's existing role gate. |

No existing hosted denial is relaxed. `HostedDenial::Fleet` on `StartFleetPolling`,
`StopFleetPolling` (`src/protocol/v1/mutation.rs:1402`) and `RequestFederationSnapshot` (`:1485`),
and `HostedDenial::ContextSwitch` on `SetContext` for any context other than `home` (`:1382`), all
stand exactly as they are. A test asserts each of those four still denies after this feature.

## UI contract

- Every surface identifying a member shows display name **and** context name, and the context name
  stays copyable verbatim (FR-002).
- Every aggregate is rendered with its coverage. A rollup with `members_covered <
  members_total` names the gaps and their reasons (FR-010, SC-004).
- `NotPermitted`, `Unreachable` and `Absent` are three distinct renderings (FR-011).
- A filter states how many of how many members it is showing (FR-016).
- Filtering a 100-member fleet responds to input in under 100 ms (SC-007, Principle V).
