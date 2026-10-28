# Contract: keel-protocol additions (spec 060)

These are additions to `crates/keel-protocol`, and nothing existing changes shape. After a change,
regenerate the TypeScript bindings in `apps/keel/web/src/protocol/generated` with
`make keel bindings`; the build fails while the committed
bindings are stale.

## Commands (`Cmd`)

Every command below is for sessions only; tokens are refused in `dispatch.rs`.

| Command | Who | Reply |
|---|---|---|
| `GetSettings` | instance admin | `Settings` |
| `PreviewSettingsUpdate { template: String, instructions: String, version: i64 }` | instance admin | `Preview` (step-up). `Confirm` replies `Settings`, or `OpError { code: "settings_conflict" }` |
| `ListEnrollments { state: Option<EnrollmentState>, workspace: Option<Uuid> }` | instance admin | `Enrollments` |
| `PreviewEnrollmentRevoke { enrollment: Uuid }` | instance admin | `Preview` (step-up). `Confirm` replies `Enrollments` |
| `PreviewEnrollmentRoll { enrollment: Uuid }` | instance admin | `Preview` (step-up). `Confirm` replies `HostEnrollment` |
| `ListAllApiTokens { holder: Option<Uuid>, state: Option<ApiTokenState> }` | instance admin | `AllApiTokens` |
| `GetSystem` | instance owner | `System` |
| `PreviewSystemConfigure { allowed_profiles: HostProfiles, version: i64 }` | instance owner | `Preview` (step-up). `Confirm` replies `System` |
| `PreviewSystemRemove` | instance owner | `Preview` (step-up). `Confirm` replies `System` |

Changed behaviour of existing commands:

- **`PreviewApiTokenRevoke`**: requires step-up when the caller doesn't own the token.
- **`PreviewUserRole`**: refused with `OpError { code: "role_managed_by_idp" }` for users whose
  `role_source` is `idp`.
- **`PreviewWorkspaceCreate`**: refuses the slug `keel-system`.
- **`PreviewGrantAdd` and `PreviewApiTokenCreate`**: refused on the System workspace
  (`Denied { reason: "owner_only" }`).
- **System workspace, any command**: refused for non-owners with
  `Denied { reason: "owner_only" }`, through `authz::require`.

## Events (`Event`)

```text
Settings {
  template: String, instructions: String, version: i64,
  is_default: bool,
  runner_address: Option<String>,      // what %s renders to
  public_url: String,                  // what %u renders to
  role_mapping: Option<RoleMappingInfo>,
  warnings: Vec<String>,               // e.g. "idp_names_no_owner"
}
RoleMappingInfo { provider: String, claim: String, owner: Vec<String>, admin: Vec<String>,
                  user: Vec<String>, required: bool }

Enrollments { items: Vec<EnrollmentInfo> }
EnrollmentInfo {
  id: Uuid, workspace: Uuid, workspace_slug: String, inventory_name: String,
  profiles: Vec<String>, state: EnrollmentState, created_by: Option<String>,
  created_at: String, expires_at: String, host: Option<Uuid>, rolled_from: Option<Uuid>,
}
EnrollmentState = "pending" | "used" | "revoked" | "burned" | "expired"

AllApiTokens { items: Vec<AdminApiTokenInfo> }
AdminApiTokenInfo { token: ApiTokenInfo, holder: Uuid, holder_name: String,
                    workspace_slug: String, state: ApiTokenState }
ApiTokenState = "active" | "expired" | "revoked"

System {
  configured: bool,
  workspace: Option<Uuid>, allowed_profiles: Vec<String>, version: i64,
  bindings: Vec<Uuid>, hosts: Vec<Uuid>,
}
```

Changed events:

- **`HostEnrollment`** gains:
  - `snippet: String`, the server-rendered template (FR-004);
  - `instructions: String`, raw Markdown, empty when unset;
  - `instructions_unavailable: bool`, true when the settings couldn't be read and the defaults were
    used.
- **`UserInfo`** gains `role_source: "keel" | "idp"`.
- **`WorkspaceInfo`** gains `system: bool`.

## Runner channel (`keel-exec` frames)

- **No new frame.** The runner replays an undelivered `Finished` after `Hello` is accepted.
- **Server handling**: the server answers a replayed `Finished` as it answers a live one.
  - For a `run_hosts` row in `lost` with the reason `runner_lost`, it records the outcome and audits
    `run_result_late`.
  - For a row already finished, it acknowledges and ignores the frame.
