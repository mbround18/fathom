# Protocol contract: Keel run approval

All changes are additive to `crates/keel-protocol`. ts-rs regenerates the TypeScript copies in
`apps/keel/web/src/protocol/generated`.

## Commands (`cmd.rs`, under "Runs (US2)")

```rust
/// `Preview` (step-up) of approving a held run. `Confirm` replies `Run`.
PreviewRunApprove { run: Uuid },
/// `Preview` (step-up) of denying a held run. `Confirm` replies `Run`.
PreviewRunDeny { run: Uuid, #[serde(default)] reason: Option<ApprovalReason> },
/// `PendingApprovals`: per workspace, the held runs the caller could approve now.
CountPendingApprovals,
```

- **Action names**: `preview_run_approve`, `preview_run_deny`, `count_pending_approvals`.
- **Capability**: `Approve` in the run's workspace, checked at preview and rechecked at confirm.
  `CountPendingApprovals` checks it per workspace, and silently skips workspaces where the caller
  lacks it.
- **Preview refusals**:
  - `run_answered`, `approval_expired`, `four_eyes_requester`, `already_answered` (see
    data-model.md);
  - plus `not_found` and `forbidden`.
- The same checks run again inside the confirm transaction (R3).
- `ApprovalReason` is a validated string of 1–500 characters, trimmed. Validate it like
  `ReviewComment`.

## `ListRuns`

When `state = pending_approval`, the result is ordered `created_at asc` (oldest first). Every
other case is unchanged.

## Types (`run.rs`)

```rust
pub struct RunInfo {
    // … existing fields …
    /// Present once the run has been held for approval.
    #[serde(default)]
    pub approval: Option<RunApproval>,
}

pub struct RunApproval {
    pub needed: u16,
    pub four_eyes: bool,
    pub expires_at: String,              // RFC 3339
    pub approvals: Vec<ApprovalEntry>,   // oldest first
    pub denial: Option<ApprovalEntry>,   // with `reason`
}

pub struct ApprovalEntry {
    pub user: Uuid,
    pub name: String,                    // display name; "Deleted user" if gone
    pub role: Option<String>,
    pub at: String,
    pub reason: Option<String>,
}
```

## Events (`event.rs`)

```rust
/// Reply to `CountPendingApprovals`.
PendingApprovals { counts: Vec<WorkspaceCount> },   // WorkspaceCount { workspace: Uuid, count: u32 }
```

The existing `RunState` event is pushed for every approval-driven transition (queued, denied,
expired). The web app refetches the count when one arrives.
