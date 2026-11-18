# Contract: keel-protocol and keel-exec additions (spec 068)

All types are additive, `#[derive(Serialize, Deserialize, TS)]`, snake_case, with ts-rs bindings
regenerated (`make -C apps/keel bindings`). Every command and event gets a serde round-trip test.

## keel-protocol (`crates/keel-protocol/src/converge.rs`, new)

```rust
pub struct ConvergeTarget { pub binding_id: Uuid, pub hosts: TargetSelection, pub check: bool }

pub struct PlannedStep { pub position: u16, pub job: String, pub via: Vec<String>,
                         pub params: BTreeMap<String, String>, pub credential_names: Vec<String> }

pub struct HostPlan { pub host_id: Uuid, pub host: String, pub profiles: Vec<String>,
                      pub steps: Vec<PlannedStep>,
                      pub refusals: Vec<StepRefusal> }   // e.g. step outside the host's ceiling
pub struct StepRefusal { pub job: String, pub reason: StepRefusalReason }
pub enum StepRefusalReason { OutsideCeiling, RunnerTooOld, HostRevoked, NoSteps }

pub struct ConvergePlan {
    pub binding_id: Uuid, pub commit: String, pub inventory_revision: Uuid,
    pub check: bool, pub hosts: Vec<HostPlan>,
    pub policy: ApprovalTerms,            // combined (strictest across steps)
    pub review: Vec<ReviewGroupSummary>,  // signed mode, per child run
    pub replaces: Vec<Uuid>,              // unapproved converges this one supersedes
    pub blocked_by: Vec<Uuid>,            // approved/running converges that refuse it
}

pub enum ConvergeHostState { Waiting, Holding, Succeeded, Failed, Cancelled }
pub enum StepState { Blocked, Queued, Sent, Running, Succeeded, Failed, Refused, Lost,
                     Cancelled, Skipped, Covered }
pub struct ConvergeStepView { pub position: u16, pub job: String, pub run_id: Uuid,
                              pub state: StepState, pub reason: Option<String>,
                              pub started_at: Option<DateTime>, pub ended_at: Option<DateTime> }
pub struct ConvergeHostView { pub host_id: Uuid, pub host: String, pub state: ConvergeHostState,
                              pub steps: Vec<ConvergeStepView> }
pub struct ConvergeView { pub run: RunSummary, pub hosts: Vec<ConvergeHostView>,
                          pub retry_of: Option<Uuid> }

pub struct ProfileSuggestion { pub host_id: Uuid, pub profile: String,
                               pub action: SuggestionAction, pub groups: Vec<String>,
                               pub first_seen_at: DateTime }
pub enum SuggestionAction { Add, Remove }

pub struct HostProfilesChange { pub host_id: Uuid, pub add: Vec<String>, pub remove: Vec<String> }
pub struct HostProfilesPreview { pub change: HostProfilesChange,
                                 pub outside_ceiling: Vec<String>,     // assigned but the runner won't accept
                                 pub principals_after: Vec<String>,
                                 pub plan_after: Vec<PlannedStep> }

pub struct RequiredCheck { pub context: String }
pub enum CheckState { Pending, Success, Failure, Error, Missing }
pub struct RunCheckView { pub context: String, pub state: CheckState, pub target_url: Option<String> }
```

### Changed types

- `RunKind` gains `Converge`.
- `RunState` gains `WaitingChecks`; `RunStateReason` gains `ChecksFailed`, `ChecksTimeout`,
  `Replaced`, `RunnerTooOld`.
- `RunHostState` gains `Blocked`, `Skipped`, `Covered`; `RunHostView` gains
  `covered_by: Option<Uuid>`, `skip_reason: Option<String>`.
- `RunSummary` gains `check: bool`, `parent_run_id: Option<Uuid>`.
- `PolicyMatch` (keel-core) gains `check: Option<bool>`.
- `ParamKind` (keel-core) gains `Choice { values: Option<Vec<String>>, from: Option<String> }`;
  `ParamView` gains `allowed: Option<Vec<String>>` (resolved at the pinned commit).
- `CredentialKind` gains `EnvValue`.
- `PreviewRunStart` gains `params: BTreeMap<String, String>` and `check: bool` (defaults: empty,
  false; `#[serde(default)]` so old clients still parse).
- `EnrollmentSettings` / `CreateEnrollment` gains `auto_converge: bool` (default true).
- `BindingView` gains `required_checks: Vec<RequiredCheck>`, `checks_timeout_secs: u32`,
  `runners_refusing_manifest: Vec<Uuid>` (research R4).
- `HostView` gains `converge: Option<ConvergeHostSummary>` (`run_id`, `state`, `step`, `of`) and
  `suggestions: u16`.

## Commands (`cmd.rs`)

| Command | Auth | Step-up | Notes |
|---|---|---|---|
| `PreviewConverge(ConvergeTarget)` → `ConvergePlan` | Operate on every profile of every targeted host | no | Pure read. |
| `ConfirmConverge { preview_token }` → `run_id` | same | no | Preview tokens as elsewhere (hash of the plan; refuses if the plan changed). |
| `PreviewConvergeRetry { run_id }` → `ConvergePlan` | same | no | Steps not succeeded, same commit/inventory. `{ run_id, replan: true }` re-plans at the binding head. |
| `ConfirmConvergeRetry { preview_token }` → `run_id` | | | |
| `CancelConverge { run_id }` | Operate | no | Cancels blocked/queued steps; a running step finishes (as cancel today). |
| `GetConverge { run_id }` → `ConvergeView` | View | | |
| `PreviewHostProfiles(HostProfilesChange)` → `HostProfilesPreview` | workspace admin | | |
| `ConfirmHostProfiles { preview_token }` | workspace admin | **yes** | Writes `host_profiles`, re-issues cert, audits. |
| `ListProfileSuggestions { host_id? }` | View | | |
| `DismissProfileSuggestion { host_id, profile, action }` | workspace admin | no | |
| `PreviewRequiredChecks { binding_id, contexts }` / `ConfirmRequiredChecks` | workspace admin | yes | Preview refuses when the forge returns `Unsupported`. |

## Events (`event.rs`)

- `ConvergeChanged { run_id, host_id, state }` — host-level change (holding, finished).
- `RunHostChanged` (existing) carries the new states; the web derives step views from it.
- `ProfileSuggestionsChanged { binding_id, hosts }` — after a sync.
- `HostProfilesChanged { host_id, profiles }`.
- `RunChecksChanged { run_id, checks: Vec<RunCheckView> }`.

## keel-core ports

```rust
// Forge (ports.rs:218)
fn commit_statuses<'a>(&'a self, repo: &'a RepoRef, sha: &'a str)
    -> ForgeFuture<'a, Vec<ForgeStatus>>;          // Err(ForgeError::Unsupported) if none
pub struct ForgeStatus { pub context: String, pub state: CheckState, pub target_url: Option<String> }

// RepoTree
fn list_dirs(&self, path: &str) -> Result<Vec<String>, TreeError>;
```

`SecretResolver::resolve` is unchanged; `queue::secrets()` uses it for every credential.

## keel-exec frames (`crates/keel-exec/src/frames.rs`)

- `Frame::Assign` gains `#[serde(default, skip_serializing_if = "is_false")] check: bool`.
  Absent ⇒ false. An old runner (`deny_unknown_fields`) refuses an Assign with `check: true`:
  fail closed.
- `SecretItem` kind gains `env_var { name: String }`, value bytes as today. The executor checks
  `name` against `keel_core::env_names` again and sets it after `env_clear`. Old runners refuse
  the unknown kind: fail closed.
- Hello capability `manifest-v068` (067's `Hello.capabilities`). The server refuses to dispatch a
  step to a host whose session lacks it **when the binding's manifest uses a 068 key**, with
  refusal `runner_too_old`.
- `Finished` unchanged. A check run reports `Checked` in the ledger only; the wire outcome is the
  normal succeeded/failed.

## keel-trust

`ledger::Outcome::Checked`: written as the `end` line of a check run. Does not move the base.
`start` still consumes the run's challenge (replay refused).
