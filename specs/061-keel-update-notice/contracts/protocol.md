# Contract: keel-protocol and keel-exec additions (spec 061)

Everything here is additive. `PROTOCOL_VERSION` stays 1. After a change, regenerate the TypeScript
bindings with `make keel bindings`. Each new variant gets a round-trip test in
`crates/keel-protocol/tests/roundtrip_061.rs`.

## Commands (`Cmd`)

Every command below is for sessions only. API tokens are refused in `dispatch.rs authorize()`,
with `Denied { reason: "session_required" }`, in the same group as the spec 060 commands. A token
refusal of `PreviewKeelUpdate` is also audited as `keel_update_refused` (FR-019, FR-020).

| Command | Who | Reply |
|---|---|---|
| `GetKeelBuild` | anyone signed in | `KeelBuild`, shaped by role (below) |
| `GetUpdateStatus` | owner or admin | `UpdateStatus` (owner) or `UpdateStatusLine` (admin). Users get `Denied { reason: "admin_only" }` |
| `CheckForUpdates` | owner | `UpdateStatus` with `state: checking`, then pushes. `OpError { code: "update_check_off" }` when checks are off. `OpError { code: "update_check_too_soon", message: <retry time> }` within a minute of the last manual check (FR-007) |
| `DismissUpdateDot { digest: String }` | owner | `Me`. `OpError { code: "invalid_digest" }` unless it matches `sha256:<64 hex>` |
| `PreviewKeelUpdate { digest: String }` | owner | `Preview` (step-up). `Confirm` replies `Run`, for the new run or the existing one (FR-018). Refusals are in the table below |

`PreviewKeelUpdate` is refused, and every refusal is audited as `keel_update_refused`:

| Condition | Reply |
|---|---|
| Caller isn't an owner | `Denied { reason: "owner_only" }` |
| No System workspace configured | `OpError { code: "no_system_workspace" }` |
| No enrolled host in it with the `keel-server` profile, or `keel-server` not in `allowed_profiles` | `OpError { code: "no_keel_host" }` or `"profile_not_allowed"` |
| The bound repo's `keel-server` job doesn't declare `params.keel_server_image_digest` | `OpError { code: "job_missing_param" }` |
| `digest` isn't the current `status.available.digest`, or the outcome isn't `update_available`, `rebuilt` or `older_build` | `OpError { code: "digest_stale" }` |
| An update run is already pending, queued or running | No refusal: the preview says so, and `Confirm` replies `Run` with the existing run (FR-018). Audited as `keel_update_refused { reason: update_in_progress }` |

The preview's text names:

- the old and new version, revision and digest;
- the workspace, host and job;
- the approval policy that will apply.

For `older_build` and `rebuilt`, the preview starts with a warning line. On `Confirm`, the server
re-checks every condition. If the status moved on in between, it refuses with `digest_stale`.

## Events (`Event`)

```text
KeelBuild {
  version: Option<String>,
  revision: Option<String>,        // null for users
  built_at: Option<String>,        // RFC 3339; null for users
  digest: Option<String>,          // owners and admins only (FR-002); null for users
  display: String,                 // "0.2.0" | "abc1234" | "development build"
}

UpdateStatus {                     // owners only
  enabled: bool,
  image: String,                   // watched reference, never with credentials
  state: UpdateCheckState,         // "not_checked" | "checking" | "checked"
  outcome: Option<UpdateOutcome>,  // "up_to_date" | "update_available" | "older_build"
                                   // | "rebuilt" | "cant_tell"
  cant_tell_reason: Option<String>,
  checked_at: Option<String>,
  next_check_at: Option<String>,
  manual_after: Option<String>,
  last_error: Option<UpdateError>, // { code: String, at: String }; code is
                                   // oci_registry::RegistryError::code() (contracts/registry.md)
  running: KeelBuild,
  available: Option<AvailableImage>,
  digest_compare: bool,            // false: running digest unknown, compared by revision (R3)
  update_path: UpdatePath,
  update_run: Option<Uuid>,        // a pending, queued or running update run
  last_update_run: Option<LastUpdateRun>,  // the latest update run that ended, so a failed
                                   // update survives a reload
}
LastUpdateRun { run: Uuid, outcome: String,      // the run's end state: "succeeded" | "failed" | "refused"
                                                 // | "cancelled" | "denied" | "expired"
                digest: String,                  // what it updated to
                ended_at: Option<String> }       // RFC 3339
AvailableImage { digest: String, version: Option<String>, revision: Option<String>,
                 built_at: Option<String>, release_notes: Option<String>,
                 release_notes_note: Option<String> }   // why there's no link (R7)
UpdatePath =
    { kind: "run" }                                     // FR-016 prerequisites met
  | { kind: "manual", missing: String,                 // "no_system_workspace" | "no_keel_host"
                                                        // | "profile_not_allowed" | "job_missing_param"
      digest_ref: String,                              // "mbround18/keel@sha256:…"
      steps: Vec<String> }                             // FR-021, rendered by the server

UpdateStatusLine {                 // admins who aren't owners (FR-015, SC-006)
  update_available: bool,          // true only for outcome update_available
  checked_at: Option<String>,
}                                  // nothing about the available image: no digest, version,
                                   // revision or date
```

The server pushes `UpdateStatus` or `UpdateStatusLine` to every connected socket whose session is
an owner or admin at push time, whenever the status changes (R5). Users never receive either.

`GetKeelBuild` is how admins get the running build for the System tab (FR-002, FR-024). The
spec 060 System commands (`GetSystem`, `PreviewSystemConfigure`, `PreviewSystemRemove`) are
unchanged and stay owner-only; the web never sends them for an admin.

Changed events and types:

- **`Me`** gains:
  - `keel_version: Option<String>`, the display string, for everyone;
  - `update_dot_dismissed: Option<String>`, owners only.
- **`RunInfo`** gains `params: BTreeMap<String, String>`, which is empty when there are none.

## Runner channel (`keel-exec` frames)

`Frame::Assign` gains one field:

```rust
#[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
params: BTreeMap<String, String>,
```

- **Without params**, an `Assign` is byte-identical to today's, so it works with old runners.
- **With params**, an old runner rejects the frame (`deny_unknown_fields`) and the run fails with
  `runner_outdated`. That is the intended fail-closed path.
- **A new runner**:
  1. re-reads the job at the pinned commit;
  2. refuses (`run_refused { reason: "params_mismatch" }`) unless every sent name is declared and
     every value matches its kind;
  3. writes `params.json` (mode 0600) and passes it as `-e @params.json` after `secret_vars.yaml`
     and before `forced.json`.

**Signed approvals**: `OverrideScope` in `crates/keel-trust/src/challenge.rs` gains
`params: BTreeMap<String, String>`. When non-empty, it is a `params` member of the JCS object the
challenge hashes. When empty, the member is omitted, so every existing challenge hashes exactly as
today (`keel-override-v1` is unchanged). The host rebuilds it from the params it was assigned.

## Manifest (`.keel/jobs.yaml`, keel-core)

```yaml
jobs:
  keel-server:
    playbook: playbooks/keel-server.yaml
    profiles: [keel-server]
    inventory: fleet
    params:
      keel_server_image_digest: { kind: oci_digest }
```

The parse rules are in data-model.md (Job parameters). `RawJob` stays `deny_unknown_fields`. An
older Keel or runner refuses a manifest that has `params:` (see the ordering in quickstart.md).
