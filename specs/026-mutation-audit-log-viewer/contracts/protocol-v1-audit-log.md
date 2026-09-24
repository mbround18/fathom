# Contract: Protocol v1 additions for the Mutation Audit-Log Viewer

All additions live in `src/protocol/v1/` and follow the existing versioned command/event contract
(Constitution Principle II). No existing command or event is removed or changed shape; this is a
purely additive protocol change, so it does not require a protocol major version bump.

## New types (`src/protocol/v1/audit.rs`, new file)

```rust
/// One completed mutating action, persisted to ~/.fathom/audit-log.json.
pub struct AuditLogEntry {
    pub id: String,
    pub timestamp: String, // RFC 3339 UTC
    pub action: AuditActionKind,
    pub action_label: String,
    pub context: String,
    pub target_kind: Option<String>,
    pub target_namespace: Option<String>,
    pub target_name: Option<String>,
    pub outcome: AuditOutcome,
    pub request_id: Option<String>,
}

pub enum AuditOutcome {
    Success,
    Failure { reason: String },
}

/// Exhaustive, no-wildcard-arm classification, mirroring `MutationTarget`'s style.
pub enum AuditActionKind {
    YamlApply,
    HelmUpgrade,
    HelmRollback,
    HelmUninstall,
    NodeCordon,
    NodeUncordon,
    NodeDrain,
    NodeTaint,
    RbacChange,
    Delete,
    BulkDelete,
    Scale,
    Restart,
    PortForwardChange,
    McpToggle,
    TeamProfilePublish,
    ContextConfigChange,
    // ... finalized during implementation to exhaustively cover mutating BackendCmdV1 variants
}
```

## New command (`src/protocol/v1/cmd.rs`)

```rust
enum BackendCmdV1 {
    // ...existing variants...

    /// Read-only: re-load the persisted audit log (bounded to the retention limit) and emit it
    /// as a ResourceList for the audit-log section.
    RefreshAuditLog,
}
```

- `mutation_target()` for `RefreshAuditLog` → `MutationTarget::ReadOnly` (it never mutates
  anything; it only reads the local file).
- `section()` for `RefreshAuditLog` → new `SectionKey::audit_log()` (`"audit-log"`), `Global`
  scope, `context: None` (the list spans all contexts by design — FR-003/User Story 1).

## New response shape

`RefreshAuditLog` is handled like other read-only list commands: the handler loads the bounded
entry list and emits the existing `BackendEventV1::ResourceList`-style event (or the closest
existing generic list-event variant used by other non-Kubernetes-native lists, e.g. events/
integrations), carrying `Vec<AuditLogEntry>`, routed via `Route::global(SectionKey::audit_log())`.

No new event variant is required if an existing generic "list of serializable items" event
already exists and can carry `AuditLogEntry`; if not, add exactly one new `BackendEventV1`
variant, e.g. `AuditLogLoaded(Vec<AuditLogEntry>)`, following the same additive-only rule.

## Internal (non-protocol) audit-write hook

Not part of the wire contract (backend-internal only), but documented here since it's the
integration point:

```rust
// src/backend.rs, inside run_cmd, before `cmd` is moved into dispatch_cmd:
let audit_summary = cmd.audit_summary(); // new method, mirrors mutation_target()

// ...after dispatch_cmd returns and `ok`/`error` are known...
if let Some(summary) = audit_summary {
    // best-effort; errors are logged via tracing::warn!, never surfaced to the command's own
    // ok/error or allowed to block/delay CommandFinished emission (FR-011).
    audit_log::record(summary, ok, error.clone(), context_name, request_id);
}
```

`cmd.audit_summary() -> Option<AuditActionSummary>` returns `None` for non-mutating commands
(`ReadOnly`, `ContextSwitch`) so they are never persisted, and `Some(..)` for every
`MutationTarget::{Namespaced, ClusterScoped, Local}` command, carrying the `AuditActionKind`,
label, and target kind/namespace/name extracted from the command's own fields.

## Backward compatibility

- Existing clients ignoring `RefreshAuditLog`/`AuditLogLoaded` are unaffected (additive).
- `~/.fathom/audit-log.json` is a new file; its absence is treated as an empty log (consistent
  with every other `~/.fathom/*.json` loader's "missing file → defaults" behavior, Constitution
  Principle VI).
