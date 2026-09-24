# Feature Specification: Hosted Server Mode (In-Cluster Web App with OIDC Login)

**Feature Branch**: `031-hosted-server-mode`

**Created**: 2026-09-23

**Status**: Draft

**Input**: User description: "offer deploying Fathom into a cluster by itself so it runs there too, letting users log in via OAuth/OIDC with RBAC nicely configured — as a web app alongside the desktop app."

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Sign in and see/do only what my RBAC allows (Priority: P1)

A team member opens the hosted Fathom URL in a browser, is redirected to their organization's identity provider, signs in, and lands in the familiar Fathom interface showing the cluster Fathom is running in. Everything they can see or change is exactly what the cluster's own access rules grant to their identity; controls for actions they are not permitted to perform are hidden or disabled, just as in the desktop app.

**Why this priority**: This is the core value: a shared, browser-accessible Fathom where per-user access is enforced by the cluster rather than by Fathom itself. Without correct per-user identity the feature is unsafe to ship.

**Independent Test**: Deploy against a cluster with an identity provider and two users holding different permissions (e.g. read-only in one namespace vs. editor in another). Sign in as each and confirm each sees and can do only what their permissions allow, including in interactive shells and port forwarding.

**Acceptance Scenarios**:

1. **Given** an unauthenticated visitor, **When** they open the hosted URL, **Then** they are sent to the identity provider to sign in and, on success, return to the app signed in, with no credentials handled by Fathom's own forms.
2. **Given** a signed-in user with read-only access to a namespace, **When** they browse it, **Then** they see its resources and mutation controls are hidden or disabled; any mutation attempted anyway is refused by the cluster and the refusal is shown clearly.
3. **Given** a signed-in user with edit access to a namespace, **When** they modify a resource there, **Then** the change succeeds, and the same action in a namespace they lack access to is refused.
4. **Given** a user who lacks permission to open a shell in or port-forward to a workload, **When** they attempt it, **Then** it is refused; a user who has the permission can do it.
5. **Given** two users signed in at once, **When** each browses, **Then** neither sees data or state belonging to the other's session, including selections, filters, and in-flight operations.
6. **Given** a signed-in user, **When** they choose to sign out, **Then** their session ends everywhere in the browser and live views stop updating; returning requires signing in again.
7. **Given** the hosted mode is running, **When** the user views the app, **Then** features that only make sense on a local desktop (in-app local cluster provisioning, choosing local kubeconfig files) are not offered.

---

### User Story 2 - Operator deploys Fathom with one command and configures OIDC (Priority: P1)

A cluster operator installs Fathom into a cluster with a single documented command, supplying their identity provider's details (issuer, client ID and secret, external URL, and how to map identity-provider groups to cluster access). After the command completes, users can reach the login page and sign in. A ready-made test overlay lets the operator try the whole flow on a local kind cluster with a bundled identity provider before going to production.

**Why this priority**: Without a simple install-and-configure path, hosted mode cannot be adopted. It is co-equal with sign-in because neither is useful alone.

**Independent Test**: On a clean kind cluster, run the documented install with the test identity provider overlay, open the URL, sign in as each sample user, and confirm the outcomes of Story 1. Then re-run with invalid OIDC settings and confirm the failure is reported clearly.

**Acceptance Scenarios**:

1. **Given** a cluster and a set of OIDC settings, **When** the operator runs the single documented install command, **Then** Fathom becomes reachable at the configured external address over an encrypted connection and users can sign in.
2. **Given** invalid or unreachable OIDC settings, **When** Fathom starts, **Then** it fails visibly with an actionable message (rather than starting in an unauthenticated or open state).
3. **Given** the operator has not chosen otherwise, **When** users act in the app, **Then** the cluster enforces each user's own permissions using their own identity (the default "pass-through" model).
4. **Given** the operator opts into the alternative "impersonation" model, **When** users act, **Then** actions are performed by Fathom's service identity on behalf of the user, with the identity provider's groups mapped to cluster permissions, and Fathom's own service identity is limited to impersonation rights only.
5. **Given** the test overlay is applied on a kind cluster, **When** the operator follows the quickstart, **Then** a bundled identity provider with sample users of differing permissions is available and sign-in works end to end.
6. **Given** the release artifacts, **When** an operator inspects them, **Then** a container image and an installable chart/manifest set are provided, and defaults are secure (encrypted transport, no anonymous access, least-privilege service identity).
7. **Given** an upgrade or uninstall, **When** the operator runs the documented command, **Then** the deployment updates or is removed cleanly without leaving broad cluster permissions behind.

---

### User Story 3 - Audit log attributes each change to the real signed-in user (Priority: P2)

Every change made through hosted Fathom is recorded in the existing mutation audit log attributed to the actual human who made it, not to a shared service account, so that operators and security reviewers can answer "who changed this?".

**Why this priority**: Attribution is essential for shared use, but it is only valuable after sign-in and per-user access exist.

**Independent Test**: Sign in as two different users, each performs a change; view the audit log and confirm each entry names the correct user identity, and that a refused change is recorded as refused or not recorded per existing audit rules.

**Acceptance Scenarios**:

1. **Given** a signed-in user performs a mutation, **When** they view the audit log, **Then** the entry names that user's identity (as asserted by the identity provider).
2. **Given** impersonation mode is enabled, **When** a user performs a mutation, **Then** the audit entry still names the real user, not the service identity.
3. **Given** the desktop app, **When** changes are made, **Then** existing audit behavior is unchanged.
4. **Given** the audit log in hosted mode, **When** an ordinary user views it, **Then** they can see only entries they are permitted to see, and entries cannot be edited or deleted by users.

---

### Edge Cases

- **Expired or revoked tokens**: When a user's credentials expire or are revoked mid-session, the app renews them silently where the identity provider allows; otherwise it ends the session cleanly, stops live data for that user, and prompts sign-in again without losing unsaved context where feasible. Revoked users lose access within a bounded time.
- **Identity provider outage**: New sign-ins fail with a clear message; already-signed-in users continue until their credentials can no longer be renewed, and the app never falls back to open access.
- **User with zero permissions**: Sign-in succeeds but the user sees a clear "no access" state explaining what was denied and whom to contact, not a blank or broken UI.
- **Many concurrent users**: Each session has isolated cluster connections and live watches; per-user and global limits on sessions, watches, and streams prevent one user from degrading others, with a clear message when a limit is reached.
- **Token refresh during long-lived watches and streams**: Live views, shells, logs, and port forwards continue across credential renewal, or end with a clear reconnectable state when the user's access has actually lapsed.
- **Cross-site request forgery and WebSocket origin**: State-changing requests and live connections from foreign origins or without a valid session are rejected.
- **Cookie and session handling**: Session credentials are not readable by page scripts, are sent only over encrypted connections, have bounded lifetime and idle timeout, and are invalidated on sign-out.
- **Multiple clusters**: Hosted mode targets the cluster it runs in as its single home cluster. Other clusters are out of scope for this version; if configured later they must use the same per-user access rules.
- **Group claims missing or too large**: If the identity provider does not supply the groups needed for the chosen model, the user sees an explicit explanation rather than silent under-access.
- **Reverse proxy / ingress in front**: Sign-in redirects and origin checks work correctly behind the operator's ingress using the configured external address.
- **Same user, several browser tabs**: Tabs share one session without duplicating cluster load beyond configured limits.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: The system MUST be deployable into a Kubernetes cluster and served to users as a web application over an encrypted connection, in addition to (not replacing) the desktop application, presenting the same user interface.
- **FR-002**: The system MUST require sign-in through a standards-based OpenID Connect provider using the authorization-code flow with proof-of-key protection; it MUST NOT offer anonymous access or handle user passwords itself.
- **FR-003**: The system MUST establish an isolated per-user session on sign-in, and MUST terminate it on sign-out, expiry, or revocation.
- **FR-004**: By default, the system MUST act toward the cluster with the signed-in user's own identity so that the cluster's own access rules decide what the user may see and do.
- **FR-005**: The system MUST offer an operator-selectable alternative in which actions are performed via a service identity impersonating the signed-in user and the user's groups, with that service identity holding only impersonation rights.
- **FR-006**: The system MUST NOT let any user read or act beyond what the cluster grants that user, including through interactive shells, log streaming, and port forwarding.
- **FR-007**: The UI MUST reflect the signed-in user's permissions (hiding or disabling disallowed actions) consistent with the existing permission-aware UI, and MUST show clear messages for refused actions and for zero-permission users.
- **FR-008**: The system MUST keep each user's cluster connections, live watches, and streams isolated from other users' sessions and MUST enforce configurable per-user and global limits, reporting when a limit is hit.
- **FR-009**: The system MUST renew user credentials when the identity provider permits, without interrupting live views, and MUST end the affected session cleanly and visibly when renewal is impossible.
- **FR-010**: The system MUST protect against cross-site request forgery and MUST reject live connections from disallowed origins or lacking a valid session.
- **FR-011**: Session credentials MUST be inaccessible to page scripts, transmitted only over encrypted connections, and subject to bounded absolute and idle lifetimes.
- **FR-012**: The system MUST use the cluster it runs in as its home cluster without user-supplied kubeconfig, and MUST NOT offer desktop-only features (local cluster provisioning, local kubeconfig selection) in hosted mode.
- **FR-013**: The system MUST record each mutation in the audit log attributed to the real signed-in user's identity, in both access models, and MUST prevent users from altering audit records.
- **FR-014**: The system MUST fail safe: if identity-provider configuration is missing, invalid, or unreachable at startup, it MUST refuse to serve the application rather than start without authentication.
- **FR-015**: The project MUST provide a container image and a single-command installable package (chart) for operators, configurable for the OIDC issuer, client credentials, external address, group mapping, access model, and limits, with secure defaults.
- **FR-016**: The project MUST provide a test overlay for the existing kind test cluster with a bundled identity provider and sample users of differing permissions, plus a quickstart documenting the end-to-end flow.
- **FR-017**: The system MUST NOT log or expose users' tokens or secrets in logs, audit entries, error messages, or the browser-visible interface.
- **FR-018**: The desktop application and the existing development bridge MUST continue to behave as before; hosted mode MUST NOT weaken their access model.
- **FR-020**: The system MUST be identity-provider-agnostic: any standards-compliant OIDC provider MUST work without provider-specific code paths, and the claims used for username and groups MUST be configurable per provider.
- **FR-021**: The project MUST include an automated end-to-end login test covering sign-in (authorization code with proof-of-key), credential renewal, sign-out, and expired/revoked credentials, and it MUST pass against two independent providers (Rauthy as primary, Keycloak as secondary).
- **FR-022**: The kind test overlay MUST deploy Rauthy (and optionally Keycloak) and MUST configure the kind API server to trust that provider (issuer, client ID, username and groups claims) via the kind cluster configuration, so pass-through RBAC is testable against a real cluster.
- **FR-019**: The system MUST expose health status suitable for cluster liveness/readiness checks and MUST report sign-in and session failures with actionable messages to users and operators.

### Key Entities

- **Hosted Deployment**: A running Fathom instance inside a cluster, with its external address, OIDC settings, access model, and limits.
- **User Session**: An authenticated, isolated, time-bounded association between a browser and a signed-in identity, owning that user's cluster connections, watches, and streams.
- **User Identity**: The subject and group memberships asserted by the identity provider, used for cluster access and audit attribution.
- **Access Model**: The operator's choice between pass-through (default) and impersonation for how user identity reaches the cluster.
- **Session Limits**: Configurable ceilings on concurrent sessions, watches, and streams per user and globally.
- **Audit Entry (extended)**: The existing mutation record, now carrying the real signed-in user's identity.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: An operator with OIDC details in hand can go from empty kind cluster to a working signed-in Fathom in under 15 minutes using one install command and the quickstart.
- **SC-002**: In a two-user test with differing permissions, 100% of actions (including shells and port forwards) are allowed or refused exactly as the cluster's own rules dictate for each user.
- **SC-003**: 100% of mutations made in hosted mode appear in the audit log attributed to the correct signed-in user.
- **SC-004**: With invalid OIDC configuration, 100% of startup attempts refuse to serve rather than run unauthenticated.
- **SC-005**: Revoking a user's access ends their live views and further actions within 5 minutes.
- **SC-006**: At least 25 concurrent signed-in users can browse and watch resources with no user seeing another's data and no user-visible failures attributable to another user's activity.
- **SC-007**: Credential renewal during an active live view causes no visible interruption in 95% of cases where the identity provider allows renewal.
- **SC-009**: The end-to-end login suite (sign-in, renewal, sign-out, expired and revoked credentials) passes 100% against both Rauthy and Keycloak.
- **SC-010**: On the kind overlay, pass-through RBAC is verified for real: users from the test providers are allowed or refused by the API server itself according to their groups.
- **SC-008**: A signed-in user with no permissions sees an explanatory no-access state within 3 seconds of sign-in.

## Assumptions

- The identity provider is any standards-compliant OIDC provider. OAuth2/OIDC is the supported protocol; LDAP is out of scope since most organizations are moving away from it (it can still be bridged by an OIDC-fronting provider).
- Test targets: Rauthy (self-hostable, Rust OIDC provider) is the primary and default end-to-end test provider; Keycloak is the second compatibility target so interoperability is not tied to one vendor. Dex, Entra, and Okta are documented as supported but are not test targets.
- Pass-through requires the cluster's API server to trust the same identity provider; where it does not (e.g. some managed clusters), operators use the impersonation model. This is documented, not auto-detected.
- Hosted mode serves one home cluster (the one it runs in); multi-cluster hosted access is deferred.
- Exposure to the internet is the operator's responsibility via their ingress/TLS setup; Fathom expects an external HTTPS address and supports being behind a proxy.
- Session state may be held in the running instance for the first version (sign-in again after restart is acceptable); multi-replica high availability is deferred.
- The existing mutation audit log (spec 026) and permission-aware UI (spec 018) are reused; this feature extends attribution and does not redesign them.
- Target scale for the first version is tens of concurrent users per instance, not thousands.

## Out of Scope

- LDAP or SAML directly.
- Fathom-managed user, role, or group administration; RBAC is authored in the cluster.
- Multi-cluster and fleet access from hosted mode.
- Multi-replica high availability and shared session storage.
- Making hosted mode a replacement for the desktop app or altering the desktop app's access model.
- In-app local cluster provisioning and local kubeconfig management in hosted mode.
- Acting as an identity provider or issuing tokens.
