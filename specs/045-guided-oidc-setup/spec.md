# Feature Specification: Guided OIDC setup

**Feature Branch**: `045-guided-oidc-setup`

**Created**: 2026-10-01

**Status**: Draft

**Input**: User description: "lets spec out the oidc wizard" — from a UI-complexity investigation (2026-10-01) of whether parts of Fathom should sit behind guided paths. `ui/src/features/identity/IssuerSettings.tsx` is a flat six-field form whose entire validation is that three strings are non-empty; it hides `username_prefix` and `groups_prefix`, which must match the API server's configuration or every binding silently matches nothing; and no `Oidc*` command verifies anything before the config is written, so the first feedback an operator gets is a failed sign-in with four possible causes. Decisions taken with the user at spec time: the path replaces the flat form for both add and edit; it covers configuration, read-only cluster checks, an offer to create the bindings through 042's existing forms, and — for local clusters Fathom provisioned — emitting and applying the API server authentication configuration.

## Clarifications

### Session 2026-10-01

- Q: How far should the guided path reach into the cluster? → A: All the way, in three widening rings. (1) Fathom's own config: issuer, claims, prefixes, group→role map — the only writes that always happen. (2) Read-only cluster checks: read the API server's authentication configuration and compare it with the issuer being configured; never change it. (3) Writes, each separately confirmed and separately refusable: hand off to 042's role/binding forms for the mapped groups, and — only for a local cluster Fathom itself provisioned (030) — apply the API server authentication configuration.
- Q: What happens to the existing flat issuer form? → A: The guided path replaces it for both add and edit. "Add issuer…" starts it empty; an existing issuer's "Review…" starts it pre-filled at step 1, so an edit gets the same probe and test sign-in an add does. The six bare `Input` fields in `IssuerSettings.tsx` are removed; the issuer list, `SignInPanel` and `RoleMapEditor` stay.
- Q: Does this conflict with the constitution's "Wizards, onboarding flows, and abstractions that hide Kubernetes objects are out of scope" (Principle I)? → A: It is the same tension 043 recorded for the setup screen, and it is resolved the same way: every step shows the real artifact (the discovery document, the decoded token claims, the API server's actual flags, the manifest about to be applied), nothing is written without an explicit confirmation that names what is written, and the path can be left at any step with nothing changed. It sequences work the operator must do anyway; it does not hide an object. To be recorded in the plan's Complexity Tracking.
- Q: Which API server authentication style does the path target? → A: Both, chosen by what the cluster already uses. Kubernetes ≥ 1.30 prefers a structured `AuthenticationConfiguration` file (`--authentication-config`), which is **mutually exclusive** with the legacy `--oidc-*` flags — the repo's own lab script (`k8s/kind-oidc-apiserver/apply.sh`) had to delete the legacy flags to add the structured one. The path reads which is in use and writes in the same style, never both.
- Q: What does the path do on a cluster Fathom did not provision (EKS, GKE, a shared cluster)? → A: Steps 1–7 are unchanged. Step 8 is not offered: it reports what the API server's configuration would need to say, as copyable text, and says plainly that Fathom will not change a control plane it does not own.
- Q: Can Fathom always read the API server's configuration? → A: No. It is readable as the `kube-apiserver` static pod in `kube-system` on self-hosted and kind clusters, and not at all on managed control planes; RBAC may also deny it. A failed read is a stated unknown ("could not read the API server's configuration: <verbatim reason>"), never an assumed pass, and never blocks the path.
- Q: Is the test sign-in mandatory before saving? → A: No, but skipping it is explicit. "Save without testing" is available at every step and says what has not been verified. Reason: an operator configuring an issuer that is not reachable from this machine yet is a real case, and Principle I says the operator decides.
- Q: Does the path write anything before its final step? → A: No. The draft lives in UI state until the operator saves. A test sign-in uses the draft directly and does not persist an issuer, so abandoning the path leaves no half-configured issuer behind.
- Q: US4 says "map groups to roles", but the code has two unrelated things by that name — which does the path do? → A: Both, as two visibly separate steps. `FathomRoleMap` (`src/protocol/v1/oidc.rs`) is three fixed slots (`viewers_group`, `editors_group`, `admins_group`) whose doc comment says they "Gate Fathom-only features; never cluster access"; 042's `RoleAssignmentDraft` → `PreviewRoleAssignment` → `ApplyResourceYaml` is actual cluster RBAC. Step A fills the three Fathom slots from the groups the test sign-in returned. Step B offers the handoff to 042 for real bindings. They are labelled so that neither is mistaken for the other: filling the Fathom map grants nothing in the cluster, and the path must say so.
- Q: After a successful test sign-in, the operator saves. Are they now signed in? → A: Yes — one sign-in, not two. The draft sign-in holds its tokens in process memory and commits them to the credential store under the issuer id at save time. This needs a command that carries the whole draft inline, because the existing `OidcSignIn { issuer_id, method }` (`src/protocol/v1/cmd.rs:757`) resolves the issuer out of *saved* config and `store.rs` keys credentials on `fathom.oidc.<issuer_id>`. Abandoning the path still discards the tokens and writes nothing.
- Q: Where does the stepped path render? → A: A dialog, like 042's `RoleAssignmentDialog`. It is the only option that works unchanged from all three places `IssuerSettings` is mounted today — the identity chip popover (`IdentityChip.tsx:49`), the Settings pane (`PaneHost.tsx:483` via `SignInSettings`) and the first-run setup screen (`SetupSignIn.tsx:43`) — none of which can navigate away to a full-screen view without losing their own context.
- Q: Does the path run in hosted mode? → A: Everything except the kind API server write. Hosted gets steps 1–7, including the read-only API server comparison (US3), which is an ordinary namespaced read performed as the signed-in user and is not a desktop-only capability. Step 8 (US6) is not offered hosted, because the constitution's hosted-server section says a hosted build "MUST NOT expose desktop-only features (kind provisioning, kubeconfig switching, stdio MCP)" — the restriction is on kind itself and predates this feature. Hosted therefore falls back to the same copyable-text output used for any cluster Fathom does not own (FR-022). This is a change from the spec's original FR-023, which withheld the API server step from hosted entirely: hosted now gains the read.
- Q: Hosted sign-in *is* OIDC, so where does a hosted deployment's first issuer come from? → A: From the process environment the server already validates before it binds. `HostedConfig::from_env` (`server/src/config.rs:158`) requires `FATHOM_OIDC_ISSUER` and `FATHOM_OIDC_CLIENT_ID` and reads `FATHOM_OIDC_SCOPES`, the claim and prefix keys, and `FATHOM_OIDC_CLIENT_SECRET_FILE` for a mounted file. That configuration exists but never reaches `persistence::load_oidc_config()`, so the issuer list the identity UI renders is empty in a server whose own sign-in is OIDC — the operator cannot add the first issuer, because adding it is what they signed in with. 045 closes the loop: hosted projects its validated startup configuration into the issuer list as a *seeded* issuer. Operationally the seed is a mounted Secret or ConfigMap plus env overrides, so it is declared in the same manifests as the rest of the deployment.
- Q: What may an operator change about a seeded issuer? → A: Nothing, through the UI. It opens read-only and states its origin ("seeded from the server environment"), while staying fully reviewable: discovery, decoded claims and the API server comparison all run against it. Changing it means changing the mounted Secret and restarting. Reason: the running server validated that issuer before it bound a socket, and every live session's token was minted by it — editing it from inside a session it authenticated would invalidate the session doing the editing. This is Principle I honoured, not withheld: the operator sees the whole object and is told exactly where to change it.
- Q: Hosted must let an existing cluster user act as cluster admin. What guards the OIDC configuration surface there? → A: The API server does, per user, through a `SelfSubjectAccessReview` on the configuration object — not a Fathom-internal role list. The primitive already exists: `review_resource_access` (`src/backend/rbac_access.rs:29`) returns the review status verbatim and `CapabilityResult` (`src/backend/capabilities.rs:272`) already carries the API server's own `reason` when denied, fail-closed with a 60s TTL. So "who may configure identity in hosted" has exactly one answer: whoever the cluster says may, and a refusal shows the cluster's own words rather than a Fathom-invented message.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Add an identity provider from its URL alone (Priority: P1)

An operator opens Settings → Sign-in and chooses "Add issuer…". They paste the issuer URL. Fathom
fetches the provider's discovery document and shows what it found: the issuer the document declares,
the endpoints, the signing keys, the scopes and claims the provider advertises, and whether the
provider supports the device-code flow. The operator enters the client id. Nothing has been written
yet. They continue to a test sign-in.

**Why this priority**: It removes the single largest source of failure — four unverifiable strings
typed from memory — and it is the step every other story builds on. On its own it already turns
"save blind and hope" into "see the provider answer before saving".

**Independent Test**: Against a stub issuer serving `/.well-known/openid-configuration`, enter the
URL, assert the discovered endpoints and scopes are shown verbatim, enter a client id, and assert
no issuer has been written to Fathom's config at any point.

**Acceptance Scenarios**:

1. **Given** a reachable issuer URL, **When** the operator leaves the URL field, **Then** Fathom fetches the discovery document and shows the declared issuer, the authorization, token and JWKS endpoints, the advertised `scopes_supported` and `claims_supported`, and whether a device-code endpoint is present.
2. **Given** a discovery document whose `issuer` differs from the URL typed, **When** it is fetched, **Then** the path stops on that step and shows both values, because a mismatch makes every later token fail validation.
3. **Given** an unreachable URL, a TLS failure or a non-JSON response, **When** it is fetched, **Then** the verbatim transport or HTTP error is shown with the URL that was tried, and the operator can supply a private CA bundle path and retry on the same step.
4. **Given** any step of the path, **When** the operator closes it, **Then** no issuer, no claim mapping and no credential has been written.

---

### User Story 2 - See the claims a real token actually carries (Priority: P1)

Before saving, the operator runs a test sign-in from inside the path. Fathom performs the real
browser (or device-code) flow against the draft issuer, validates the ID token it gets back, and
shows the decoded claims: which claim holds the username, which holds the groups, and what the
values are for the person who just signed in. The operator picks the username and groups claims from
what the token actually contains instead of typing claim names from the provider's documentation.

**Why this priority**: Claim names are the second-largest source of silent failure, and they are
knowable only from a real token. This is also what makes step 4's prefix comparison meaningful: a
prefix is only correct relative to a real username.

**Independent Test**: With a stub IdP issuing a token containing `preferred_username` and `groups`,
run the test sign-in, assert the decoded claims are listed with their values, assert selecting a
claim updates the draft, and assert the test sign-in leaves no persisted issuer behind.

**Acceptance Scenarios**:

1. **Given** a draft issuer, **When** the operator runs the test sign-in, **Then** the existing sign-in stages (awaiting browser / device code / validating) are shown, and on success the token's claims are listed with their values.
2. **Given** a successful test sign-in, **When** the claims are shown, **Then** the username and groups claims can be chosen from the claims present in the token, and the chosen values are previewed as the username and group list Kubernetes will see, prefixes included.
3. **Given** a token with no claim matching the chosen groups claim, **When** the claims are shown, **Then** Fathom says the token carried no groups and warns that group-based bindings cannot match, without blocking.
4. **Given** a failed test sign-in, **When** the error arrives, **Then** the IdP's verbatim reason is shown and the operator can go back to any earlier step, fix it and retest.
5. **Given** a machine with no usable browser, **When** the provider advertises device-code, **Then** the test sign-in offers the device-code flow.

---

### User Story 3 - Reconcile with what the cluster will accept (Priority: P1)

The path reads how the active cluster's API server is configured to accept OIDC tokens — the issuer
URL, client id (audience), username and groups claims, and the username and groups prefixes — and
shows it beside the draft. Where they disagree, it names the exact disagreement and what it will
cause. Where it cannot read the configuration, it says so and why.

**Why this priority**: This is the failure the flat form cannot even express: Fathom's own config can
be perfect and every binding still match nothing because the API server prefixes usernames with
`oidc:` and the bindings do not. It is read-only, so it carries no risk and can ship before any
write.

**Independent Test**: Against a cluster whose `kube-apiserver` static pod carries known OIDC flags,
assert each value is read and displayed; then against a draft that disagrees on the prefix, assert
the mismatch is named and explains that bindings will not match.

**Acceptance Scenarios**:

1. **Given** a cluster whose API server is readable, **When** step 4 opens, **Then** the API server's issuer, audience, claims and prefixes are shown verbatim beside the draft's, marked agreeing or disagreeing one by one.
2. **Given** the API server uses a structured `AuthenticationConfiguration`, **When** it is read, **Then** that is what is shown and compared, and the path notes that the legacy flags are not in use.
3. **Given** the API server's configuration cannot be read (managed control plane, or RBAC denies it), **When** step 4 opens, **Then** Fathom states that it could not read it and gives the verbatim reason, and the path continues with the comparison marked unknown.
4. **Given** the draft's issuer URL or client id differs from the API server's, **When** the comparison is shown, **Then** Fathom states that tokens from this issuer will be rejected by this API server, and distinguishes that from a prefix mismatch, where tokens are accepted but bindings do not match.

---

### User Story 4 - Map groups to roles, and create the bindings (Priority: P2)

With a real group list from step 2 in hand, the operator does two separate things. First they fill
Fathom's three role slots — viewers, editors, admins — which gate Fathom's own features and grant
nothing in the cluster. Then, as its own step, they are offered the cluster bindings that would
actually grant access. Choosing to create them
hands off to the existing role and binding forms (042), which show the manifest, diff it and require
the same confirmation as any other mutation.

**Why this priority**: An issuer that is configured but grants nothing looks broken in exactly the
way this feature exists to prevent. It is P2 rather than P1 because the mapping is useful on its own
and the bindings can be created by hand today.

**Independent Test**: After a test sign-in returning groups `platform` and `dev`, assign a role to
`platform`, assert the handed-off binding form opens pre-filled with the prefixed group name as the
subject, and assert nothing is applied until that form's own confirmation.

**Acceptance Scenarios**:

1. **Given** groups from the test sign-in, **When** step 5 opens, **Then** those groups are offered as the values for the three Fathom role slots, alongside free text for groups not in this operator's own token, and the step states that these grant nothing in the cluster.
2. **Given** a group and a chosen role, **When** the operator asks for the binding, **Then** 042's binding form opens with the subject pre-filled as the group name with the API server's groups prefix applied, and the operator completes that form's existing review → confirm → apply.
3. **Given** the operator declines the bindings, **When** the path finishes, **Then** the issuer and the Fathom role slots are still saved, and the path says which groups have no cluster binding yet.
4. **Given** the active context denies creating bindings, **When** step 5 opens, **Then** the offer is gated and disabled with the API server's verbatim reason, exactly as every other mutating control.

---

### User Story 5 - Save, and reopen to review (Priority: P2)

The final step shows the complete issuer configuration as it will be stored, and what was and was
not verified. Saving writes it. Afterwards, the issuer's "Review…" reopens the same path pre-filled
at step 1, so re-probing a provider, retesting a sign-in and correcting a claim are the same actions
as configuring it the first time.

**Why this priority**: It is what lets the path replace the flat form rather than sit beside it; and
re-verification after an IdP changes is the common long-term case.

**Independent Test**: Complete the path and assert the saved config equals what the summary showed;
reopen "Review…" on that issuer and assert every step is pre-filled from the saved config and that
leaving without saving changes nothing.

**Acceptance Scenarios**:

1. **Given** a completed path, **When** the summary is shown, **Then** it lists every stored field including the prefixes, and states which steps were verified and which were skipped.
2. **Given** a saved issuer, **When** "Review…" is chosen, **Then** the path opens pre-filled at step 1 and re-probes discovery.
3. **Given** a reopened issuer whose id is changed, **When** it is saved, **Then** Fathom explains that the id keys the stored credentials and that changing it signs this issuer out, before writing.
4. **Given** an operator who wants none of the verification, **When** they choose "Save without testing", **Then** the issuer is saved from whatever the draft holds and the summary records what was not verified.

---

### User Story 6 - Configure the API server of a cluster Fathom provisioned (Priority: P3)

On a local kind cluster that Fathom created (030), the path offers a last step: apply the API server
authentication configuration that matches the issuer just configured. It shows the exact file or
flags it will write, states that the API server will restart and may be briefly unavailable, keeps a
copy of the previous manifest, and waits for the control plane to come back. On every other cluster
this step is replaced by copyable text of what the configuration would need to say.

**Why this priority**: It closes the loop for the local-cluster case Fathom owns end to end, and it
is the only step that can break a cluster, so it ships last and alone.

**Independent Test**: On a kind cluster Fathom provisioned, run the step, assert the manifest backup
exists, assert the API server returns to ready, and assert a sign-in through the configured issuer is
then accepted by the API server. Separately, on a non-provisioned context, assert the step is not
offered and the copyable text appears instead.

**Acceptance Scenarios**:

1. **Given** a kind cluster Fathom provisioned, **When** step 8 opens, **Then** the complete file or flag change is shown before anything is applied, with a confirmation that names the cluster and says the API server will restart.
2. **Given** the API server currently uses legacy `--oidc-*` flags and the cluster supports a structured configuration, **When** the change is shown, **Then** Fathom states that the two are mutually exclusive and that the legacy flags will be removed.
3. **Given** the change is applied, **When** the API server restarts, **Then** Fathom waits for readiness and reports it, and on failure shows the verbatim error together with the path of the saved previous manifest and how to restore it.
4. **Given** any context Fathom did not provision, **When** the path reaches this point, **Then** no change is offered: the required configuration is shown as copyable text with a sentence saying Fathom does not change a control plane it does not own.
5. **Given** hosted mode, **When** the path runs, **Then** this step does not exist at all.

### Edge Cases

- The provider's discovery document is reachable but its `issuer` field disagrees with the URL entered (US1 #2) — stop, show both.
- The provider is behind a private CA: discovery fails with a TLS error, and the CA bundle path must be settable and retried on the same step without losing the rest of the draft.
- The provider advertises neither the device-code endpoint nor a usable browser is present: the test sign-in cannot run; the path must still allow saving.
- The token carries groups as a single string rather than an array (the existing reader already handles both), or carries empty group names (Rauthy emits `[""]`; already dropped) — the preview must show what Kubernetes will see, not the raw claim.
- The operator's own token has no groups, so step 5 has nothing to offer: free-text group entry must still work.
- Two issuers are configured and a second is added with the same id: the id keys the credential store, so this must be refused with an explanation, not silently overwrite.
- The API server has no OIDC configuration at all: the comparison shows "not configured", and that tokens from this issuer will be rejected until it is.
- The API server is configured for a *different* issuer: say so explicitly; a cluster accepts a limited set.
- The test sign-in's token expires while the operator is still in a later step: the claims shown are a record of that sign-in, not a live session, and must be labelled with when it was taken.
- Reading the `kube-apiserver` pod succeeds but the flags are unparseable (an unusual distribution): report the raw command line rather than guessing.
- The path is open when the active context changes underneath it: the cluster comparison belongs to a named context and must either follow the change and re-read, or say which context it was taken from.
- Step 8's apply succeeds but the API server never returns to ready: the restore instructions and backup path must be shown, and Fathom must not retry on its own.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: The system MUST fetch an OIDC provider's discovery document from an issuer URL before any issuer is saved, and show the endpoints, signing-key location, advertised scopes and claims, and device-code support it declares.
- **FR-002**: The system MUST refuse to continue when the discovery document's `issuer` differs from the URL entered, showing both values.
- **FR-003**: The system MUST report discovery failures with the verbatim transport or HTTP error and the URL tried, and MUST allow a private CA bundle path to be supplied and the fetch retried without losing the rest of the draft.
- **FR-004**: The system MUST be able to run a complete sign-in against a draft issuer that has not been saved, using the existing browser and device-code flows and the existing sign-in progress stages. The draft's tokens MUST be held in process memory only, since the existing sign-in command resolves its issuer out of saved config and the credential store is keyed by issuer id.
- **FR-005**: The system MUST show the validated ID token's claims with their values, and MUST let the username and groups claims be chosen from the claims the token actually carries.
- **FR-006**: The system MUST preview the username and the group list as Kubernetes will see them, with the configured prefixes applied.
- **FR-007**: The system MUST NOT persist an issuer, a credential or a claim mapping at any step before the operator saves; abandoning the path MUST discard the draft's tokens and leave no trace.
- **FR-008**: The system MUST read the active cluster's API server authentication configuration — both the legacy `--oidc-*` flags and a structured `AuthenticationConfiguration` — and display it verbatim beside the draft.
- **FR-009**: The system MUST compare issuer URL, client id/audience, username claim, groups claim, username prefix and groups prefix one by one, and MUST distinguish a disagreement that causes tokens to be *rejected* from one that causes bindings to *not match*.
- **FR-010**: The system MUST state when the API server's configuration could not be read, with the verbatim reason, and MUST NOT treat an unreadable configuration as agreement.
- **FR-011**: The system MUST expose the username and groups prefixes as editable fields with their current values, since they decide whether bindings match.
- **FR-012**: The system MUST offer the groups observed in the test sign-in, plus free text, as the values of the three Fathom role slots (`viewers_group`, `editors_group`, `admins_group`), and MUST save them with the issuer. It MUST state that these gate Fathom's own features and grant nothing in the cluster.
- **FR-013**: The system MUST be able to hand a mapped group to the existing role-binding form with the subject pre-filled including the API server's groups prefix, and MUST NOT create any binding except through that form's existing review, confirmation and gating.
- **FR-014**: The system MUST show a final summary of every field to be stored, including which verification steps were performed and which were skipped, before writing.
- **FR-015**: Users MUST be able to save at any step without completing the verification, and the summary MUST record what was not verified.
- **FR-016**: The system MUST open the same path, pre-filled, when an existing issuer is reviewed, and MUST warn before saving a change to an issuer id that it signs that issuer out because the id keys the stored credentials.
- **FR-017**: The system MUST refuse to save an issuer whose id is already in use, with an explanation.
- **FR-018**: The system MUST replace the existing flat issuer form for both adding and editing; the issuer list, the per-issuer sign-in controls and the role-map editor remain.
- **FR-019**: On a local cluster Fathom provisioned, the system MUST be able to apply the API server authentication configuration matching the issuer, and MUST show the complete change, name the cluster, and require a confirmation stating that the API server will restart.
- **FR-020**: The system MUST write the legacy flags or the structured configuration in whichever style the cluster already uses, MUST NOT produce both, and MUST state when adding the structured configuration requires removing the legacy flags.
- **FR-021**: The system MUST keep a copy of the previous API server manifest before changing it, MUST wait for the control plane to become ready afterwards, and on failure MUST show the verbatim error, the backup's location and how to restore it, without retrying on its own.
- **FR-022**: The system MUST NOT offer to change the API server of any context Fathom did not provision, and MUST instead show the required configuration as copyable text.
- **FR-023**: The system MUST offer the read-only API server comparison (FR-008–FR-010) in hosted mode, performed as the signed-in user, and MUST NOT offer the API server write (FR-019) there; hosted falls back to the copyable-text output of FR-022. Rationale: the constitution forbids a hosted build from exposing kind provisioning.
- **FR-024**: Every cluster write the path performs MUST appear in the mutation audit log like any other write.
- **FR-025**: Each step MUST be reachable backwards without losing the draft, and the path MUST be closable at every step.
- **FR-026**: The path MUST state which context the cluster comparison was read from, and MUST re-read or say so when the active context changes while it is open.
- **FR-027**: The Fathom role slots (FR-012) and the cluster bindings (FR-013) MUST be presented as two separate, separately refusable steps, and the path MUST NOT imply that filling the Fathom slots granted any cluster access.
- **FR-028**: On saving after a successful test sign-in, the system MUST establish the operator's session from that sign-in's tokens without requiring a second sign-in.
- **FR-029**: The path MUST render as a dialog, openable unchanged from the identity chip, the Settings sign-in pane and the first-run setup screen.
- **FR-030**: The binding handoff MUST carry the id of the issuer being configured, not the first saved issuer. (`RoleAssignmentLauncher.tsx:19` reads `issuers[0]?.id` today, which is wrong as soon as a second issuer exists.)
- **FR-031**: In hosted mode the system MUST present the server's validated startup OIDC configuration as an issuer in the issuer list, so the path's steps 1–7 operate on the issuer the deployment actually uses rather than on an empty list.
- **FR-032**: A seeded issuer MUST state that it came from the server environment and MUST NOT be editable or deletable through the guided path, while remaining fully reviewable — discovery, claims and the API server comparison all available — and the path MUST name where it is changed instead.
- **FR-033**: The hosted seed MUST be supplied through the process environment from a mounted Secret or ConfigMap, and those objects MUST appear in manifest export like every other object Fathom would apply. No secret value is ever rendered in the UI or in an export.
- **FR-034**: In hosted mode, access to the OIDC configuration surface MUST be decided by a `SelfSubjectAccessReview` performed as the signed-in user against the configuration object, never by a Fathom-internal role list; when the review denies, the system MUST show the API server's verbatim reason.
- **FR-035**: When that review cannot be completed, the system MUST fail closed and state the unknown verbatim, in the same shape as the API server read of FR-010.
- **FR-036**: The system MUST record, when it provisions a local cluster, that Fathom provisioned it, and MUST base the ownership test of FR-019 and FR-022 on that record. A kind cluster with no such record MUST be treated as not owned. (Nothing is persisted today and `KindClusterSummary.profile` is hardcoded `None` at `src/backend/kind.rs:225`, so FR-019 is not satisfiable without this.)

### Key Entities

- **Issuer draft**: the in-progress configuration — id, issuer URL, client id, scopes, CA bundle path, username and groups claims, username and groups prefixes, optional admin API base. Lives only in UI state until saved; becomes the existing stored issuer config on save.
- **Discovery result**: what the provider's `/.well-known/openid-configuration` declared — declared issuer, endpoints, JWKS location, supported scopes, supported claims, device-code support. Evidence, not configuration; never stored.
- **Token claims sample**: the claims of one validated ID token from the test sign-in, with the time it was taken. Evidence; never stored.
- **API server authentication**: what the cluster's control plane is configured to accept — style (legacy flags or structured), issuer, audience, claims, prefixes — or the reason it could not be read. Read-only except in US6.
- **Group→role mapping**: which IdP groups map to which roles, saved with the issuer; the source of the bindings offered in US4.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: An operator who knows only their provider's URL and client id can configure a working sign-in without consulting the provider's documentation for claim names.
- **SC-002**: Every one of the four causes of a failed sign-in (unreachable or mismatched issuer, wrong client id, wrong claim name, wrong prefix) is distinguishable from the path's output alone, without reading a log.
- **SC-003**: A prefix mismatch between Fathom and the API server is reported before the issuer is saved, in 100% of cases where the API server's configuration is readable.
- **SC-004**: Abandoning the path at any step leaves Fathom's stored configuration byte-identical to what it was before the path opened.
- **SC-005**: Configuring an issuer end to end, including a test sign-in, takes one pass with no step that requires information from outside Fathom other than the issuer URL and client id.
- **SC-006**: On a Fathom-provisioned local cluster, an operator can go from no OIDC at all to a successful `kubectl`-equivalent authenticated request through the UI, without editing a file by hand.
- **SC-007**: No path step writes to Fathom's config or the cluster without a confirmation that names what is written.

## Assumptions

- The provider is a public client using PKCE; no client secret is ever configured. This matches the existing issuer model and is not revisited here.
- Discovery, token validation, the browser and device-code flows and claim decoding already exist in the backend and are reused rather than rebuilt; what is new is reaching them before an issuer is saved, and a read of the API server's configuration.
- Reading the API server's configuration means reading the `kube-apiserver` static pod in `kube-system`. Managed control planes do not expose it, and RBAC may deny it; both are stated unknowns, not failures.
- "A cluster Fathom provisioned" means one carrying the provenance record FR-036 introduces. 030 as built persists nothing about the clusters it creates — `kind.rs` derives its list from `kind get clusters` and sets `profile: None` — so this test cannot be answered today, and FR-036 is a prerequisite of FR-019 rather than a detail of it. Treating every `kind-*` context as owned was rejected: it would write to a control plane the operator built by hand, which is the one thing FR-022 exists to prevent. Hosted builds never provision kind clusters, so the test is always false there.
- The Fathom role slots and the cluster bindings the path suggests both operate on groups, not individual users; per-user bindings remain available through the existing forms.
- The cluster bindings step runs after the save, because 042's `RoleAssignmentDraft` requires a saved `issuer_id`. The path's save is therefore the boundary between "nothing written" and "bindings offered".
- The hosted seed reuses the server's existing startup configuration rather than introducing a second source of truth; `HostedConfig.oidc`/`claims` and `OidcIssuerConfig` already carry the same fields under different names, and bridging them is a projection, not a new format.
- Multi-issuer setups keep working: the path configures one issuer at a time and never alters another.
- This feature does not change how sign-in works once configured, how sessions refresh, or how the per-context auth mode is chosen.
- The constitution's Principle I tension ("wizards… are out of scope") is to be recorded in the plan's Complexity Tracking, with the same justification the setup screen used in 043.
