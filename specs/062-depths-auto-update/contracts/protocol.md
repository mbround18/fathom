# Contract: protocol changes (062, `crates/depths-protocol`)

All new variants:
- get serialisation round-trip tests;
- get ts-rs bindings, regenerated and drift-checked;
- bump `PROTOCOL_VERSION` (to the next number after 054's bump), because `StackDetail`,
  `AuditEntry`, `Provider` and `Cmd` change. New fields are optional on the wire.

Data shapes (`ServiceUpdate`, `UpdatePolicy`, `Window`, `UpdateRun`, `AuditUpdate`, …) are in
[../data-model.md](../data-model.md).

## Types

```rust
pub struct Window { pub days: Vec<Weekday>, pub start: String /* HH:MM */, pub end: String }
pub enum Weekday { Mon, Tue, Wed, Thu, Fri, Sat, Sun }

pub struct UpdateSettings {
    pub checks_enabled: bool, pub interval_secs: u32, pub window: Window,
    pub scope: Option<String>, pub label_only: bool, pub cleanup: bool,
    pub health_timeout_secs: u32,
}

pub struct UpdatesView {
    pub settings: UpdateSettings,
    pub available: UpdatesAvailability,       // Off | On | Unreadable { path, error }
    pub zone: String,                         // "Europe/Berlin (UTC+01:00)" or "UTC (no /etc/localtime)"
    pub next_check: Option<String>,
    pub next_window: Option<String>,
    pub auto_paused: Option<AutoPause>,       // WatchtowerRunning { containers } | Disabled | SettingsUnreadable
    pub services: Vec<ServiceUpdate>,
    pub policies: BTreeMap<String, StackUpdatePolicy>,
    pub recent_runs: Vec<UpdateRun>,
    pub watchtower: Vec<WatchtowerContainer>,
    pub registries: Vec<RegistryStatus>,      // host, auth = anonymous|config|helper_missing, backoff_until
}
pub struct WatchtowerContainer { pub id: String, pub name: String, pub image: String,
                                 pub acknowledged: Option<WatchtowerAnswer> }
pub enum WatchtowerAnswer { Stopped, MonitorOnly }

/// One stack in a batch preview.
pub struct StackUpdatePlan {
    pub stack: String,
    pub services: Vec<ServiceUpdate>,         // only those that will be recreated
    pub skipped: Vec<(String, String)>,       // service, reason (up to date, excluded, rejected…)
    pub argv: Vec<Vec<String>>,               // the steps, as every preview shows them
    pub findings: Vec<Finding>,               // 051 lint findings for the stack
    pub ack_required: bool,
}

pub enum UpdateChange {
    Settings(UpdateSettings),
    Policy { stack: String, policy: UpdatePolicy, window: Option<Window> },
    AckWatchtower { container_id: String, answer: WatchtowerAnswer },
    Retry { stack: String, service: String, digest: String },
    ApplyHeld { stack: String, service: String, digest: String },
}
```

`Provider` gains `Depths`. `AuditEntry` gains `on_behalf_of`, `update`, `change`.
`AuditAction` gains the variants listed in the data model. `StackDetail` gains
`updates: Option<Vec<ServiceUpdate>>` (`None` when checks are off or the feature is disabled).

## Cmd

| Command | Reply | Notes |
|---|---|---|
| `ListUpdates` | `Updates(UpdatesView)`, then live `UpdatesChanged(UpdatesView)` | Admin. Subscribing is a `Subscribe`-kind command; closing the view unsubscribes. |
| `CheckUpdatesNow { stacks: Option<Vec<String>> }` | `CheckStarted` \| `Refused::Busy` (rate limit, `retry_after_secs`) | Admin. At most one per minute host-wide (FR-002). Refused with `Invalid` while checks are off. |
| `PreviewUpdateBatch { stacks: Vec<String> }` | `UpdatePreview { preview_id, stacks: Vec<StackUpdatePlan>, step_up }` | Admin. Binds each stack's `content_hash`, the digests shown and the order. |
| `PreviewUpdateChange(UpdateChange)` | `Preview` with `step_up` and a before/after summary | Admin. FR-017. |
| `Confirm { preview_id, acknowledge_findings, assertion }` | Batch: `UpdateRunStarted { run_id }`, then per stack `OpStarted` / `OpOutput`* / `OpFinished`, `UpdateStackDone`, and `UpdateRunFinished`. Change: `UpdatesChanged`. | The existing `Confirm`; preview IDs are unique across kinds. `acknowledge_findings` covers 051 warnings in the batch. |
| `Cancel { operation_id }` | existing | Cancels the stack in progress; the batch stops there and the rest are `Skipped { reason: "cancelled" }`. |

User path (US2 scenario 5): users keep 050's `PreviewOp { op: Update }` on assigned stacks. Its
plan becomes the same per-stack image-update plan (pull by digest, hold check, tag, `up --no-deps`)
when checks are on and the stack has update states; with checks off it stays 050's pull-then-up.

## Events

| Event | When |
|---|---|
| `UpdatesChanged(UpdatesView)` | After a check pass, a run step, a settings change, a Watchtower container appearing or going. Coalesced: at most one per second. |
| `UpdatesBadge { stacks_with_updates: u32 }` | To admins, on sign-in and when the count changes. Drives the navigation badge without loading the whole view. |
| `UpdateRunStarted { run_id, trigger, stacks }` | A batch or scheduled run starts. |
| `UpdateStackDone { run_id, stack, outcome, audit_at }` | Each stack finishes. |
| `UpdateRunFinished { run_id, summary: Vec<(String, StackOutcome)> }` | The batch summary (US2). |
| `StackChanged` (existing) | Carries the stack's new `updates` states to users on that stack. |

Scheduled runs send the same events to admins who are connected.

## Gates

In order, as for every command (054 contracts): authenticate → network trust (052) →
verification → role/stack access → step-up → handler.

| Command | `CommandKind` | Role | Trust | Step-up |
|---|---|---|---|---|
| `ListUpdates` | `ViewUpdates` (new) | admin | — | — |
| `CheckUpdatesNow` | `CheckUpdates` (new) | admin | required (makes outbound requests) | — |
| `PreviewUpdateBatch` / its `Confirm` | `Update` per stack, plus admin | admin | required | yes (054 risky-action rule) |
| `PreviewUpdateChange` / its `Confirm` | `ManageUpdates` (new) | admin | required | yes, always (FR-017) |
| `PreviewOp { Update }` (user) | `Update` | user on assigned stack | required | as 054 |

Scheduled runs have no session. They are authorised by the policy itself: setting a stack to Auto
needed a trusted network and a passkey, and the run names that administrator (`on_behalf_of`). An
admin who is removed or demoted: their Auto policies are paused (shown "author no longer an
administrator") until another admin re-confirms them.

## Refusals and holds

- Stale: the stack's `content_hash` changed since the preview → that stack is
  `Skipped { reason: "stale: preview again" }`; the rest continue.
- Busy: another operation on the stack → `Skipped { reason: "busy: <actor> <kind>" }` (US2 sc. 2).
- Hold: `Held` with `reason` = the 051 code or the guard rule; the stack's audit entry is
  `image_hold`.
- A guard refusal from `up` (054 `GuardRefusal` on `OpFinished`) is recorded as `Held`, not
  `Failed`, when the old container is still running.

## 054 contract amendment

054's guard allowlist (contracts/guard.md) must add `POST /images/{name}/tag` (used for the tag and
rollback steps, research R2/R3). `POST /images/create` with `fromImage=<repo>@<digest>` is already
covered by image create.
