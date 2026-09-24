# Phase 1 Data Model: Gatekeeper Policy Pane

All types live in `src/protocol/v1/gatekeeper.rs` (new file), derive
`Serialize, Deserialize, TS` (the standard protocol-type derive set — see `src/protocol/v1/flux.rs`
for the precedent), and are code-generated into `ui/src/protocol/generated/*.ts` by the existing
`ts-rs` build step. No new derive macros or codegen configuration are introduced.

## GatekeeperInstallState

Distinguishes the four honest states required by FR-002 / User Story 3. Kept as its own enum
(rather than overloading `installed: bool` the way `FluxOverview` does) because this feature has
one more distinct state than Flux does (permission-denied), and collapsing four states into a
bool-plus-flags shape would be less clear than Flux's simpler two-state case.

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
pub enum GatekeeperInstallState {
    /// Neither `constrainttemplates.templates.gatekeeper.sh` nor any
    /// `constraints.gatekeeper.sh`-group CRD was found.
    NotInstalled,
    /// Gatekeeper's CRDs are present but zero `Constraint` instances exist.
    InstalledNoConstraints,
    /// Gatekeeper's CRDs are present and at least one `Constraint` instance exists.
    Active,
    /// CRDs could not be listed due to an RBAC/permission error (FR-002, User Story 3
    /// acceptance scenario 3). Carries the verbatim backend error per Principle I.
    PermissionDenied,
}
```

## GatekeeperViolation

One entry from a `Constraint`'s `.status.violations[]`, enriched with a resolved Inspector
navigation target (see research.md R4).

```rust
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct GatekeeperViolation {
    /// Where to navigate this object in the Inspector (research.md R4); `custom_resources()`
    /// fallback for kinds with no dedicated section.
    pub section: SectionKey,
    pub kind: String,
    pub namespace: String,
    pub name: String,
    /// `enforcementAction` reported on this specific violation entry (Gatekeeper allows a
    /// constraint's overall enforcementAction to be overridden per-violation in some template
    /// designs); falls back to the constraint's own enforcement_action when absent.
    pub enforcement_action: String,
    pub message: String,
}
```

## GatekeeperConstraintSummary

One `Constraint` instance (of any constraint kind).

```rust
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct GatekeeperConstraintSummary {
    /// The constraint kind (== the CRD's Kind, e.g. "K8sRequiredLabels"), NOT the literal
    /// string "Constraint" — mirrors `FluxResourceSummary.kind` distinguishing concrete kinds.
    pub kind: String,
    pub name: String,
    /// `spec.enforcementAction` (e.g. "deny", "dryrun", "warn"); empty string if unset
    /// (Gatekeeper defaults to "deny" but we show what's actually on the object, not an
    /// inferred default, per Principle I's "show what's happening" spirit).
    pub enforcement_action: String,
    /// `.status.totalViolations` if present, else `.status.violations.len()`, else `None`
    /// when `.status` itself is absent (the "awaiting status" edge case).
    pub total_violations: Option<u64>,
    /// Full detail for up to Gatekeeper's configured `constraintViolationsLimit` violations.
    pub violations: Vec<GatekeeperViolation>,
    /// True when `total_violations > violations.len()` — drives the "showing N of M" truncation
    /// notice (FR-007).
    pub violations_truncated: bool,
    /// True when the constraint's `spec.constraintTemplate`-equivalent template CRD could not
    /// be resolved (orphaned constraint edge case) — informational only, does not block display.
    pub template_missing: bool,
}
```

## GatekeeperOverview

Top-level aggregate, one instance of which is broadcast per poll — the direct analog of
`FluxOverview`.

```rust
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct GatekeeperOverview {
    pub state: GatekeeperInstallState,
    /// Verbatim backend error text when `state == PermissionDenied`; empty otherwise.
    pub error: String,
    pub constraints: Vec<GatekeeperConstraintSummary>,
}
```

## Protocol wiring (existing files, additive changes only)

- `src/protocol/v1/cmd.rs`: add `BackendCmd::RefreshGatekeeper` (unit variant, mirrors
  `RefreshFlux`/`RefreshHubble`).
- `src/protocol/v1/event.rs`: add `SystemEvent::GatekeeperOverviewUpdated(GatekeeperOverview)`.
- `src/protocol/v1/mutation.rs`: classify `RefreshGatekeeper` alongside `RefreshFlux`/`RefreshHubble`
  in the read-only command list, and route it to `section_keys::integrations_gatekeeper()`.
- `src/protocol/v1/route.rs`: add `section_keys::integrations_gatekeeper() -> SectionKey`
  (`"integrations/gatekeeper"`), placed next to `integrations_flux()`/`integrations_hubble()`.
- `src/protocol/v1/mod.rs`: re-export the new `gatekeeper` module's public types (mirrors how
  `flux` is re-exported).

## Frontend state (Zustand store, `ui/src/store/index.ts`)

```ts
gatekeeper: {
  overview: GatekeeperOverview | null
}
refreshGatekeeper: () => void   // sends BackendCmd::RefreshGatekeeper, mirrors refreshFlux
```

`SystemEvent::GatekeeperOverviewUpdated` updates `gatekeeper.overview` in the store's event
reducer, exactly where `FluxOverviewUpdated` updates `flux.overview` today.

## Validation rules (from Requirements)

- FR-004: a constraint with `.status` entirely absent MUST render `total_violations: None`, never
  `Some(0)` — the frontend renders `None` as an explicit "awaiting status" tone/label, distinct
  from a real zero.
- FR-007: `violations_truncated` MUST be computed purely from `total_violations` vs.
  `violations.len()`, never inferred from a fixed limit constant, since Gatekeeper's
  `constraintViolationsLimit` is operator-configurable.
- FR-009: no command in this feature's protocol surface may appear in `mutation.rs`'s mutating
  classification; both `RefreshGatekeeper` and reads of `GatekeeperOverview`/`GatekeeperViolation`
  are strictly informational.
