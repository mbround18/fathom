//! Client → server frames (contracts/protocol.md, "Envelopes" and "Commands").
//!
//! Parsing is not authorisation: the server authorises each command against the session's
//! current `authz_epoch` at the time of the command.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use keel_core::uuid::Uuid;

use crate::binding::{
    BindingSpec, BindingUpdate, CredentialKind, DeliveryDecision, Instant, ObjectName,
    ReceiverProvider, SecretValue,
};
use crate::passkey::{PasskeyAssertion, PasskeyPurpose, PasskeyResponse, RecoveryCode};
use crate::types::{B64Url, ClientName, CsrfToken, PreviewId, RequestId};
// Inventories (US4)
use crate::inventory::InventoryText;
// Governance (US6)
use crate::governance::{GrantProfiles, Label, Slug, TokenActions};
use crate::types::{InstanceRole, TeamRole};
// Runs (US2)
use crate::run::{Pattern, PolicySpec, RunPhase};

/// The first frame on a WebSocket. A wrong `csrf_token` closes with 4403; a missing or expired
/// session with 4401; another `protocol_version` with 4400.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct Hello {
    /// [`crate::PROTOCOL_VERSION`] the client was built against.
    pub protocol_version: u32,
    /// Free-form client name and version, for the audit context.
    pub client: ClientName,
    /// The session's CSRF token (from the `__Host-keel_csrf` cookie).
    pub csrf_token: CsrfToken,
}

/// Every frame after `Hello`, and the body of `POST /api/v1/cmd`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct CmdEnvelope {
    pub id: RequestId,
    pub cmd: Cmd,
}

/// Commands. Results arrive as events.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "type", content = "data", deny_unknown_fields)]
pub enum Cmd {
    /// `Me`.
    GetMe,
    // Bindings and receivers (US1)
    /// `Bindings`: the workspace's repository bindings.
    ListBindings {
        #[ts(type = "string")]
        workspace: Uuid,
    },
    /// `Binding`: one binding with its jobs.
    GetBinding {
        #[ts(type = "string")]
        binding: Uuid,
    },
    /// `Preview` after a test fetch (with signed mode, it lists unsigned recent commits).
    /// `Confirm` replies `Binding`.
    PreviewBindingCreate(BindingSpec),
    /// `Preview`; `Confirm` replies `Binding`.
    PreviewBindingUpdate(BindingUpdate),
    /// `Preview`; turning signed mode off needs step-up. `Confirm` replies `Binding`.
    PreviewSignedMode {
        #[ts(type = "string")]
        binding: Uuid,
        on: bool,
    },
    /// `Preview` listing the queued and pending runs it cancels (`binding_disabled`).
    PreviewBindingDisable {
        #[ts(type = "string")]
        binding: Uuid,
    },
    /// `Preview`; `Confirm` replies `Binding`.
    PreviewBindingEnable {
        #[ts(type = "string")]
        binding: Uuid,
    },
    /// `Preview`; `Confirm` removes the binding, its receiver links and its mirror.
    PreviewBindingRemove {
        #[ts(type = "string")]
        binding: Uuid,
    },
    /// Fetch now. Replies `BindingSynced`, or `OpError`.
    SyncBinding {
        #[ts(type = "string")]
        binding: Uuid,
    },
    /// `Receivers`.
    ListReceivers {
        #[ts(type = "string")]
        workspace: Uuid,
    },
    /// `Preview` (step-up); `Confirm` replies `ReceiverSecret` once.
    PreviewReceiverCreate {
        #[ts(type = "string")]
        workspace: Uuid,
        name: ObjectName,
        provider: ReceiverProvider,
    },
    /// `Preview` (step-up); `Confirm` replies `ReceiverSecret` once. The previous secret keeps
    /// verifying for `previous_valid_for` seconds (default 24 h).
    PreviewReceiverRotate {
        #[ts(type = "string")]
        receiver: Uuid,
        #[serde(default)]
        previous_valid_for: Option<u32>,
    },
    /// `Preview`; `Confirm` replies `Receivers`.
    PreviewReceiverDisable {
        #[ts(type = "string")]
        receiver: Uuid,
    },
    /// `Preview`; `Confirm` replies `Receivers`.
    PreviewReceiverRemove {
        #[ts(type = "string")]
        receiver: Uuid,
    },
    /// `Preview` (step-up); `Confirm` replies `Receivers`.
    PreviewReceiverBind {
        #[ts(type = "string")]
        receiver: Uuid,
        #[ts(type = "string")]
        binding: Uuid,
    },
    /// `Preview`; `Confirm` replies `Receivers`.
    PreviewReceiverUnbind {
        #[ts(type = "string")]
        receiver: Uuid,
        #[ts(type = "string")]
        binding: Uuid,
    },
    /// `Deliveries`, newest first (at most 200).
    ListDeliveries {
        #[ts(type = "string")]
        workspace: Uuid,
        #[serde(default)]
        #[ts(type = "string | null")]
        receiver: Option<Uuid>,
        #[serde(default)]
        #[ts(type = "string | null")]
        binding: Option<Uuid>,
        #[serde(default)]
        decision: Option<DeliveryDecision>,
        #[serde(default)]
        from: Option<Instant>,
        #[serde(default)]
        to: Option<Instant>,
    },
    /// `Credentials`: names, kinds, fingerprints and grants. Never values.
    ListCredentials {
        #[ts(type = "string")]
        workspace: Uuid,
    },
    /// `Preview` (step-up); `Confirm` replies `Credentials`.
    PreviewCredentialCreate {
        #[ts(type = "string")]
        workspace: Uuid,
        name: ObjectName,
        kind: CredentialKind,
        value: SecretValue,
    },
    /// `Preview` (step-up); `Confirm` replies `Credentials`.
    PreviewCredentialRotate {
        #[ts(type = "string")]
        credential: Uuid,
        value: SecretValue,
    },
    /// `Preview` (step-up). Empty `profiles`: every profile. `Confirm` replies `Credentials`.
    PreviewCredentialGrant {
        #[ts(type = "string")]
        credential: Uuid,
        #[ts(type = "string")]
        binding: Uuid,
        profiles: Vec<ObjectName>,
    },
    /// `Preview` (step-up); `Confirm` replies `Credentials`.
    PreviewCredentialRevoke {
        #[ts(type = "string")]
        credential: Uuid,
        #[ts(type = "string")]
        binding: Uuid,
    },
    /// `Preview`, or `OpError {credential_in_use}` while a pending or queued run needs it.
    PreviewCredentialRemove {
        #[ts(type = "string")]
        credential: Uuid,
    },
    // Inventories (US4)
    /// `Inventories`.
    ListInventories {
        #[ts(type = "string")]
        workspace: Uuid,
    },
    /// `InventoryRevision`: revision `seq` (the newest when absent), with the history.
    GetInventoryRevision {
        #[ts(type = "string")]
        inventory: Uuid,
        #[serde(default)]
        seq: Option<u32>,
    },
    /// `InventoryDiff` from revision `from` to revision `to`.
    DiffInventory {
        #[ts(type = "string")]
        inventory: Uuid,
        from: u32,
        to: u32,
    },
    /// `Preview` of a new managed inventory, or `OpError {inventory_invalid}`. `var_schema` is a
    /// JSON Schema (as JSON text) for each host's vars. Repository inventories come from the
    /// manifest. `Confirm` replies `InventoryRevision`.
    PreviewInventoryCreate {
        #[ts(type = "string")]
        workspace: Uuid,
        name: ObjectName,
        content: InventoryText,
        #[serde(default)]
        var_schema: Option<InventoryText>,
    },
    /// `Preview` of a new revision of a managed inventory. `OpError {stale_base}` unless
    /// `base_seq` is the newest; `{inventory_invalid}` when the content fails validation;
    /// `{no_change}` when it equals the newest. `Confirm` replies `InventoryRevision`.
    PreviewInventoryEdit {
        #[ts(type = "string")]
        inventory: Uuid,
        base_seq: u32,
        content: InventoryText,
    },
    /// `Preview` of a new revision equal to revision `seq`. `Confirm` replies `InventoryRevision`.
    PreviewInventoryRestore {
        #[ts(type = "string")]
        inventory: Uuid,
        seq: u32,
    },
    /// `Preview`; a deprecated inventory accepts no new runs. `Confirm` replies `Inventories`.
    PreviewInventoryDeprecate {
        #[ts(type = "string")]
        inventory: Uuid,
    },
    /// `InventoryValidation`: a dry run that stores nothing. With `inventory`, its var schema
    /// applies; a draft `var_schema` (JSON text) applies instead, and its own problems are
    /// reported under `schema`.
    ValidateInventory {
        #[ts(type = "string")]
        workspace: Uuid,
        content: InventoryText,
        #[serde(default)]
        #[ts(type = "string | null")]
        inventory: Option<Uuid>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        var_schema: Option<InventoryText>,
    },
    /// `Preview` of a managed inventory's var schema; `None` removes it. `OpError
    /// {inventory_invalid}` when the newest revision fails it, `{schema_invalid}` when it is not a
    /// JSON object, `{no_change}` when it equals the current one. `Confirm` replies
    /// `InventoryRevision`.
    PreviewInventorySchema {
        #[ts(type = "string")]
        inventory: Uuid,
        #[serde(default)]
        var_schema: Option<InventoryText>,
    },
    /// Carry out a `Preview`. `step_up` is required when the preview said so and the session has
    /// not verified within the step-up window. Replies `Confirmed` plus the command's own result.
    Confirm {
        preview_id: PreviewId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        step_up: Option<PasskeyAssertion>,
    },

    // Passkeys, as in Depths (spec 054). Session callers only.
    /// `Verification`.
    GetVerification,
    /// `PasskeyChallenge`, answered with `PasskeyFinish`.
    PasskeyBegin { purpose: PasskeyPurpose },
    /// `PasskeyRegistered` (then `Verification`), `Verification`, or `PasskeyRefused`.
    PasskeyFinish(PasskeyResponse),
    /// `Passkeys`: the person's own.
    ListPasskeys,
    /// `Preview` (always `step_up`); `Confirm` removes it and ends the person's other sessions.
    /// `PasskeyRefused {last_passkey}` for the last one.
    PreviewPasskeyRemove { id: B64Url },
    /// `Verification` (with `recovering`), or `PasskeyRefused {recovery_invalid}`. Removes every
    /// passkey and ends the person's other sessions.
    RecoverPasskeys { code: RecoveryCode },

    // Governance (US6). Instance-wide commands are for instance owners and admins, sessions
    // only; workspace-scoped ones need `ws_admin` there. Confirms reply the updated list.
    /// `Workspaces`: every workspace for instance admins, otherwise the ones the caller holds a
    /// grant on (a token: its own).
    ListWorkspaces,
    /// `Preview`. Instance admins.
    PreviewWorkspaceCreate { slug: Slug, name: Label },
    /// `Preview`: rename. `ws_admin` on the workspace.
    PreviewWorkspaceUpdate {
        #[ts(type = "string")]
        workspace: Uuid,
        name: Label,
    },
    /// `Preview` naming everyone who loses access. Instance admins; narrowing.
    PreviewWorkspaceArchive {
        #[ts(type = "string")]
        workspace: Uuid,
    },
    /// `Preview`. `ws_admin` on the workspace.
    PreviewProjectCreate {
        #[ts(type = "string")]
        workspace: Uuid,
        slug: Slug,
        name: Label,
    },
    /// `Teams`: every team for instance admins, otherwise the caller's own teams and those
    /// granted on workspaces the caller administers.
    ListTeams,
    /// `Preview`. Instance admins.
    PreviewTeamCreate { name: Label },
    /// `Preview` naming the user and what they gain. Instance admins.
    PreviewTeamMemberAdd {
        #[ts(type = "string")]
        team: Uuid,
        #[ts(type = "string")]
        user: Uuid,
    },
    /// `Preview`. Instance admins; narrowing.
    PreviewTeamMemberRemove {
        #[ts(type = "string")]
        team: Uuid,
        #[ts(type = "string")]
        user: Uuid,
    },
    /// `Preview` (step-up) listing the team's members. `ws_admin` on the workspace. Empty
    /// `profiles` grants the whole workspace.
    PreviewGrantAdd {
        #[ts(type = "string")]
        team: Uuid,
        #[ts(type = "string")]
        workspace: Uuid,
        role: TeamRole,
        #[serde(default)]
        profiles: GrantProfiles,
    },
    /// `Preview` listing the team's members. `ws_admin` on the grant's workspace; narrowing.
    PreviewGrantRemove {
        #[ts(type = "string")]
        grant: Uuid,
    },
    /// `Users`. Instance admins.
    ListUsers,
    /// `Preview` (step-up). Instance admins; only an owner makes or unmakes an owner. The last
    /// owner cannot be demoted.
    PreviewUserRole {
        #[ts(type = "string")]
        user: Uuid,
        role: InstanceRole,
    },
    /// `Preview` (step-up): disables the user, ends their sessions, drops their memberships and
    /// revokes their tokens. Instance admins; only an owner removes an owner; never the last
    /// owner, never oneself.
    PreviewUserRemove {
        #[ts(type = "string")]
        user: Uuid,
    },
    /// `ApiTokens`: the caller's own. Sessions only.
    ListApiTokens,
    /// `Preview` (step-up); `Confirm` replies `ApiTokenCreated` with the secret, once. The token
    /// acts as the caller, narrowed to `workspace` and `actions`. Sessions only.
    PreviewApiTokenCreate {
        name: Label,
        #[ts(type = "string")]
        workspace: Uuid,
        actions: TokenActions,
        /// 1 to 366.
        expires_in_days: u16,
    },
    /// `Preview`. The token's owner, or an instance admin. Sessions only.
    PreviewApiTokenRevoke {
        #[ts(type = "string")]
        token: Uuid,
    },
    // Runs (US2)
    /// `Runs`: the workspace's runs, newest first (at most 200), filtered.
    ListRuns {
        #[ts(type = "string")]
        workspace: Uuid,
        #[serde(default)]
        #[ts(type = "string | null")]
        binding: Option<Uuid>,
        /// A host's inventory name.
        #[serde(default)]
        host: Option<ObjectName>,
        #[serde(default)]
        #[ts(type = "string | null")]
        actor: Option<Uuid>,
        #[serde(default)]
        state: Option<RunPhase>,
        #[serde(default)]
        from: Option<Instant>,
        #[serde(default)]
        to: Option<Instant>,
    },
    /// `Run`: one run with its hosts.
    GetRun {
        #[ts(type = "string")]
        run: Uuid,
    },
    /// `RunSubscribed`, then the run's stored `RunEvent`/`RunLog` after `from_seq` in order,
    /// then live ones until the run ends or `UnsubscribeRun`. WebSocket only.
    SubscribeRun {
        #[ts(type = "string")]
        run: Uuid,
        #[serde(default)]
        from_seq: u64,
    },
    UnsubscribeRun {
        #[ts(type = "string")]
        run: Uuid,
    },
    /// `Preview` naming the pinned commit, inventory revision and hosts; `Confirm` replies `Run`.
    PreviewRunStart {
        #[ts(type = "string")]
        job: Uuid,
        #[serde(default)]
        limit: Option<Pattern>,
    },
    /// `Preview`; `Confirm` sends `Cancel` to the run's hosts and replies `RunState`.
    PreviewCancel {
        #[ts(type = "string")]
        run: Uuid,
    },
    /// `Jobs`: the binding's current jobs.
    ListJobs {
        #[ts(type = "string")]
        binding: Uuid,
    },
    /// `Policies`: the workspace's approval policies, by priority.
    ListPolicies {
        #[ts(type = "string")]
        workspace: Uuid,
    },
    /// `Preview` (step-up); `Confirm` replies `Policies`. The catch-all cannot be narrowed.
    PreviewPolicyUpsert(PolicySpec),
    /// `Preview` (step-up); `Confirm` replies `Policies`. The catch-all cannot be removed.
    PreviewPolicyRemove {
        #[ts(type = "string")]
        policy: Uuid,
    },
    /// `PolicyExplanation`: which policy a run of `job` with `limit` would get, and why.
    ExplainPolicy {
        #[ts(type = "string")]
        job: Uuid,
        #[serde(default)]
        limit: Option<Pattern>,
    },
}

/// The commands an API token may be scoped to, by [`Cmd::name`]: reads only, for now. Anything
/// else, including any command added later, is refused to tokens until it is listed here. A
/// write belongs here only by a deliberate choice; passkey and token management never do.
pub const TOKEN_ACTIONS: &[&str] = &[
    "get_me",
    // Bindings and receivers (US1)
    "list_bindings",
    "get_binding",
    "list_receivers",
    "list_deliveries",
    "list_credentials",
    // Inventories (US4)
    "list_inventories",
    "get_inventory_revision",
    "diff_inventory",
    "validate_inventory",
    // Governance (US6)
    "list_workspaces",
    "list_teams",
    "list_users",
    // Runs (US2)
    "list_runs",
    "get_run",
    "list_jobs",
    "list_policies",
    "explain_policy",
];

impl Cmd {
    /// The command's name, as audited and as `Denied.action` reports it.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Cmd::GetMe => "get_me",
            // Bindings and receivers (US1)
            Cmd::ListBindings { .. } => "list_bindings",
            Cmd::GetBinding { .. } => "get_binding",
            Cmd::PreviewBindingCreate(_) => "preview_binding_create",
            Cmd::PreviewBindingUpdate(_) => "preview_binding_update",
            Cmd::PreviewSignedMode { .. } => "preview_signed_mode",
            Cmd::PreviewBindingDisable { .. } => "preview_binding_disable",
            Cmd::PreviewBindingEnable { .. } => "preview_binding_enable",
            Cmd::PreviewBindingRemove { .. } => "preview_binding_remove",
            Cmd::SyncBinding { .. } => "sync_binding",
            Cmd::ListReceivers { .. } => "list_receivers",
            Cmd::PreviewReceiverCreate { .. } => "preview_receiver_create",
            Cmd::PreviewReceiverRotate { .. } => "preview_receiver_rotate",
            Cmd::PreviewReceiverDisable { .. } => "preview_receiver_disable",
            Cmd::PreviewReceiverRemove { .. } => "preview_receiver_remove",
            Cmd::PreviewReceiverBind { .. } => "preview_receiver_bind",
            Cmd::PreviewReceiverUnbind { .. } => "preview_receiver_unbind",
            Cmd::ListDeliveries { .. } => "list_deliveries",
            Cmd::ListCredentials { .. } => "list_credentials",
            Cmd::PreviewCredentialCreate { .. } => "preview_credential_create",
            Cmd::PreviewCredentialRotate { .. } => "preview_credential_rotate",
            Cmd::PreviewCredentialGrant { .. } => "preview_credential_grant",
            Cmd::PreviewCredentialRevoke { .. } => "preview_credential_revoke",
            Cmd::PreviewCredentialRemove { .. } => "preview_credential_remove",
            // Inventories (US4)
            Cmd::ListInventories { .. } => "list_inventories",
            Cmd::GetInventoryRevision { .. } => "get_inventory_revision",
            Cmd::DiffInventory { .. } => "diff_inventory",
            Cmd::PreviewInventoryCreate { .. } => "preview_inventory_create",
            Cmd::PreviewInventoryEdit { .. } => "preview_inventory_edit",
            Cmd::PreviewInventoryRestore { .. } => "preview_inventory_restore",
            Cmd::PreviewInventoryDeprecate { .. } => "preview_inventory_deprecate",
            Cmd::ValidateInventory { .. } => "validate_inventory",
            Cmd::PreviewInventorySchema { .. } => "preview_inventory_schema",
            Cmd::Confirm { .. } => "confirm",
            Cmd::GetVerification => "get_verification",
            Cmd::PasskeyBegin { .. } => "passkey_begin",
            Cmd::PasskeyFinish(_) => "passkey_finish",
            Cmd::ListPasskeys => "list_passkeys",
            Cmd::PreviewPasskeyRemove { .. } => "preview_passkey_remove",
            Cmd::RecoverPasskeys { .. } => "recover_passkeys",
            // Governance (US6)
            Cmd::ListWorkspaces => "list_workspaces",
            Cmd::PreviewWorkspaceCreate { .. } => "preview_workspace_create",
            Cmd::PreviewWorkspaceUpdate { .. } => "preview_workspace_update",
            Cmd::PreviewWorkspaceArchive { .. } => "preview_workspace_archive",
            Cmd::PreviewProjectCreate { .. } => "preview_project_create",
            Cmd::ListTeams => "list_teams",
            Cmd::PreviewTeamCreate { .. } => "preview_team_create",
            Cmd::PreviewTeamMemberAdd { .. } => "preview_team_member_add",
            Cmd::PreviewTeamMemberRemove { .. } => "preview_team_member_remove",
            Cmd::PreviewGrantAdd { .. } => "preview_grant_add",
            Cmd::PreviewGrantRemove { .. } => "preview_grant_remove",
            Cmd::ListUsers => "list_users",
            Cmd::PreviewUserRole { .. } => "preview_user_role",
            Cmd::PreviewUserRemove { .. } => "preview_user_remove",
            Cmd::ListApiTokens => "list_api_tokens",
            Cmd::PreviewApiTokenCreate { .. } => "preview_api_token_create",
            Cmd::PreviewApiTokenRevoke { .. } => "preview_api_token_revoke",
            // Runs (US2)
            Cmd::ListRuns { .. } => "list_runs",
            Cmd::GetRun { .. } => "get_run",
            Cmd::SubscribeRun { .. } => "subscribe_run",
            Cmd::UnsubscribeRun { .. } => "unsubscribe_run",
            Cmd::PreviewRunStart { .. } => "preview_run_start",
            Cmd::PreviewCancel { .. } => "preview_cancel",
            Cmd::ListJobs { .. } => "list_jobs",
            Cmd::ListPolicies { .. } => "list_policies",
            Cmd::PreviewPolicyUpsert(_) => "preview_policy_upsert",
            Cmd::PreviewPolicyRemove { .. } => "preview_policy_remove",
            Cmd::ExplainPolicy { .. } => "explain_policy",
        }
    }

    /// Streaming commands (subscriptions) only work over the WebSocket; `POST /api/v1/cmd`
    /// refuses them with `use_websocket`.
    #[must_use]
    pub const fn is_streaming(&self) -> bool {
        match self {
            Cmd::GetMe
            // Bindings and receivers (US1)
            | Cmd::ListBindings { .. }
            | Cmd::GetBinding { .. }
            | Cmd::PreviewBindingCreate(_)
            | Cmd::PreviewBindingUpdate(_)
            | Cmd::PreviewSignedMode { .. }
            | Cmd::PreviewBindingDisable { .. }
            | Cmd::PreviewBindingEnable { .. }
            | Cmd::PreviewBindingRemove { .. }
            | Cmd::SyncBinding { .. }
            | Cmd::ListReceivers { .. }
            | Cmd::PreviewReceiverCreate { .. }
            | Cmd::PreviewReceiverRotate { .. }
            | Cmd::PreviewReceiverDisable { .. }
            | Cmd::PreviewReceiverRemove { .. }
            | Cmd::PreviewReceiverBind { .. }
            | Cmd::PreviewReceiverUnbind { .. }
            | Cmd::ListDeliveries { .. }
            | Cmd::ListCredentials { .. }
            | Cmd::PreviewCredentialCreate { .. }
            | Cmd::PreviewCredentialRotate { .. }
            | Cmd::PreviewCredentialGrant { .. }
            | Cmd::PreviewCredentialRevoke { .. }
            | Cmd::PreviewCredentialRemove { .. }
            // Inventories (US4)
            | Cmd::ListInventories { .. }
            | Cmd::GetInventoryRevision { .. }
            | Cmd::DiffInventory { .. }
            | Cmd::PreviewInventoryCreate { .. }
            | Cmd::PreviewInventoryEdit { .. }
            | Cmd::PreviewInventoryRestore { .. }
            | Cmd::PreviewInventoryDeprecate { .. }
            | Cmd::ValidateInventory { .. }
            | Cmd::PreviewInventorySchema { .. }
            | Cmd::Confirm { .. }
            | Cmd::GetVerification
            | Cmd::PasskeyBegin { .. }
            | Cmd::PasskeyFinish(_)
            | Cmd::ListPasskeys
            | Cmd::PreviewPasskeyRemove { .. }
            | Cmd::RecoverPasskeys { .. } => false,
            // Governance (US6)
            Cmd::ListWorkspaces
            | Cmd::PreviewWorkspaceCreate { .. }
            | Cmd::PreviewWorkspaceUpdate { .. }
            | Cmd::PreviewWorkspaceArchive { .. }
            | Cmd::PreviewProjectCreate { .. }
            | Cmd::ListTeams
            | Cmd::PreviewTeamCreate { .. }
            | Cmd::PreviewTeamMemberAdd { .. }
            | Cmd::PreviewTeamMemberRemove { .. }
            | Cmd::PreviewGrantAdd { .. }
            | Cmd::PreviewGrantRemove { .. }
            | Cmd::ListUsers
            | Cmd::PreviewUserRole { .. }
            | Cmd::PreviewUserRemove { .. }
            | Cmd::ListApiTokens
            | Cmd::PreviewApiTokenCreate { .. }
            | Cmd::PreviewApiTokenRevoke { .. } => false,
            // Runs (US2)
            Cmd::SubscribeRun { .. } | Cmd::UnsubscribeRun { .. } => true,
            Cmd::ListRuns { .. }
            | Cmd::GetRun { .. }
            | Cmd::PreviewRunStart { .. }
            | Cmd::PreviewCancel { .. }
            | Cmd::ListJobs { .. }
            | Cmd::ListPolicies { .. }
            | Cmd::PreviewPolicyUpsert(_)
            | Cmd::PreviewPolicyRemove { .. }
            | Cmd::ExplainPolicy { .. } => false,
        }
    }
}
