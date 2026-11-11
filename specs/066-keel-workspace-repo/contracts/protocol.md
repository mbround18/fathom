# Protocol contract: Keel workspaces, repository catalog and hosts

All changes are additive to `crates/keel-protocol`. ts-rs regenerates
`apps/keel/web/src/protocol/generated`.

## Commands (`cmd.rs`)

Under "Workspaces":

```rust
/// `Preview` (workspace admin) of setting or clearing the primary binding. `Confirm` replies
/// `Workspaces`.
PreviewWorkspacePrimary { workspace: Uuid, binding: Option<Uuid> },
/// `Preview` (workspace admin): create a binding on the `default` project and make it primary.
/// `Confirm` replies `Binding`, then syncs. Same validation as `PreviewBindingCreate`.
PreviewWorkspaceRepository { workspace: Uuid, spec: RepositorySpec },
/// `ForgeConnection` (View).
GetForgeConnection { workspace: Uuid },
/// `Preview` (workspace admin) of setting or replacing the forge connection. `Confirm` replies
/// `ForgeConnection`.
PreviewForgeConnection { workspace: Uuid, provider: ForgeProvider, api_base: Option<ApiBase>,
                         credential: Uuid },
/// `Preview` (workspace admin) of removing it.
PreviewForgeRemove { workspace: Uuid },
/// `ForgeProbe`. Reads only.
ProbeForge { workspace: Uuid },
```

Under "Repository" (new):

```rust
/// `Catalog` for the workspace's primary binding (View), with per-host decrypt state.
GetCatalog { workspace: Uuid },
/// `JobDraft` (View). Pure: stores nothing.
DraftJob { workspace: Uuid, playbook: RepoPath },
/// `Preview` (Operate) of adding a host: enrolment plus inventory change. `Confirm` replies
/// `HostAdded`.
PreviewHostAdd { workspace: Uuid, inventory_name: InventoryName, groups: Vec<GroupName>,
                 profiles: Vec<ProfileName>, inventory: Option<RepoPath> },
/// `Preview` (workspace admin) of proposing a host's age key as a sops recipient. `Confirm`
/// replies `ChangeProposed`.
PreviewSopsRecipient { host: Uuid },
/// `Preview` (workspace admin) of proposing a job draft. `Confirm` replies `ChangeProposed`.
PreviewJobPropose { workspace: Uuid, playbook: RepoPath, name: JobName },
```

**Action names** are `snake_case` of the variant, as today:

- `preview_workspace_primary`
- `preview_workspace_repository`
- `get_forge_connection`
- `preview_forge_connection`
- `preview_forge_remove`
- `probe_forge`
- `get_catalog`
- `draft_job`
- `preview_host_add`
- `preview_sops_recipient`
- `preview_job_propose`

The mutating ones join `Cmd::is_mutating`.

**Refusal codes (new)**:

- `no_primary_repository`
- `no_catalog`
- `forge_not_configured`
- `forge_writes_disabled`
- `forge_refused`
- `host_exists`
- `group_not_found`
- `inventory_not_yaml`
- `unsupported_layout`
- `no_age_recipient`
- `no_sops_rule`
- `playbook_not_found`
- `job_exists`
- `recipient_exists`: the sops rule already lists the host's age key.
- `unparseable`: the file at the head does not parse, so no edit is computed.
- `invalid` (existing): a name, recipient or job body the edit rejects.

A rule without an `age` list (PGP only, or `key_groups` without `age`) refuses with
`unsupported_layout`.

## Events (`event.rs`)

```rust
Catalog(CatalogView),            // CatalogView = Catalog + hosts: Vec<HostDecrypt> + stale/error
JobDraft { yaml: String, notes: Vec<String> },
ForgeConnection(Option<ForgeConnectionInfo>),
ForgeProbe { ok: bool, can_push: bool, message: Option<String> },
HostAdded { enrollment: EnrollmentIssued /* as today */, change: ChangeOutcome },
ChangeProposed(ChangeOutcome),
```

`ChangeOutcome` is one of three variants:

- `Opened { url, branch }`
- `Manual { diff, reason }`
- `Failed { diff, error }`

## Types

- `WorkspaceInfo` gains `primary_binding: Option<Uuid>`.
- `HostInfo` gains `age_recipient: Option<String>` and `inventory_status: InventoryStatus`
  (`listed`, `pending_inventory` or `unknown`).
- `CredentialKind` gains `ForgeToken`.
- `Preview` gains `warnings: Vec<String>`, with `#[serde(default, skip_serializing_if =
  "Vec::is_empty")]`. `PreviewRunStart` fills it with sops warnings (FR-018).
- `RepositorySpec` is `BindingSpec` without `project`.

## Runner (`keel-exec` frames)

`Frame::Hello` gains `age_recipient: Option<String>`, with `#[serde(default,
skip_serializing_if = "Option::is_none")]`. An old runner omits it. The server stores it, or
clears it when the runner sends none.
