# Feature Specification: Uwusquad Suite Portal

**Feature Branch**: `057-uwusquad-suite-portal`

**Created**: 2026-10-26

**Status**: Draft

**Input**: User description: "all 4 apps in our roster should share code and be hyper secure ... i want to churn them into a product suite ... spec a portal app ... to marry all these apps together under uwusquad.dev"

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Sign in once and access only allowed suite actions (Priority: P1)

A platform operator signs into `uwusquad.dev` once and can reach Fathom, Depths, Keel, and Ballast from one portal. Every action is authorized by shared team and namespace/project policies, so the operator only sees and triggers what their role permits.

**Why this priority**: Shared identity, RBAC, and policy enforcement are the safety foundation for turning separate apps into one product suite.

**Independent Test**: Create owner/admin/user accounts across two teams, sign in through the portal, and confirm each account only sees and executes its allowed suite actions across all four products.

**Acceptance Scenarios**:

1. **Given** an unauthenticated visitor, **When** they open any portal or suite route, **Then** they must sign in before any data is disclosed.
2. **Given** a signed-in user with team-scoped permissions, **When** they access suite resources outside that scope, **Then** access is denied and the denial is audited.
3. **Given** role or team changes by an administrator, **When** the user sends their next request, **Then** updated policy is enforced immediately without a service restart.

---

### User Story 2 - Orchestrate cross-app workflows safely (Priority: P1)

An operator can run approved workflows that coordinate multiple products, such as provisioning infrastructure through Keel and Ballast, deploying runtime stacks through Depths, and validating outcomes in Fathom, with clear step-by-step status and rollback.

**Why this priority**: Cross-product workflow orchestration is the core suite value beyond simple navigation.

**Independent Test**: Execute a multi-step workflow spanning at least three apps, induce a controlled failure in one step, and verify deterministic rollback, final state visibility, and audit completeness.

**Acceptance Scenarios**:

1. **Given** a workflow template that spans products, **When** an operator starts it, **Then** each step runs in order with explicit status and actor attribution.
2. **Given** a workflow step failure, **When** rollback policy exists, **Then** rollback executes automatically or by operator approval and records resulting state.
3. **Given** duplicate or retried workflow triggers, **When** processed, **Then** only one effective run is executed and duplicates are safely ignored.

---

### User Story 3 - Govern teams, environments, and approvals centrally (Priority: P1)

Security administrators can manage teams, role bindings, environment scopes, and approval gates from one control surface, and those policies apply consistently to every product enrolled in the suite.

**Why this priority**: A suite-level control plane must reduce governance drift between products, not multiply it.

**Independent Test**: Configure policy once in the portal, then verify the same policy is enforced in every enrolled product without per-app manual duplication, and that a product that is not enrolled is unaffected.

**Acceptance Scenarios**:

1. **Given** a policy requiring approval for sensitive provisioning actions, **When** a non-approver triggers the action, **Then** execution pauses and awaits authorized approval.
2. **Given** a user removed from a team, **When** they attempt any suite action, **Then** active sessions are invalidated and access is refused.
3. **Given** an owner account, **When** another admin attempts to demote or remove that owner, **Then** the action is refused and audited.

---

### User Story 4 - Discover and operate a coherent product suite brand (Priority: P2)

A customer discovers uwusquad.dev as one integrated operations suite with clear product boundaries (Fathom, Depths, Keel, Ballast), shared navigation patterns, and brand-consistent language, icons, and onboarding.

**Why this priority**: A coherent suite identity improves adoption, reduces operator confusion, and supports product-led growth.

**Independent Test**: Run first-time user onboarding and confirm users can identify each product’s purpose, launch correct workflows, and complete one cross-app task without external documentation.

**Acceptance Scenarios**:

1. **Given** a first-time user, **When** they enter the portal, **Then** they see clear suite positioning and product intents before technical setup.
2. **Given** shared design primitives, **When** navigating between products, **Then** visual semantics and interaction patterns remain consistent.
3. **Given** brand assets updates, **When** published, **Then** all product entry surfaces reflect the update without inconsistent or stale variants.

---

### User Story 5 - Understand why UwU Squad exists and how to adopt it (Priority: P2)

A buyer or technical evaluator can quickly understand the suite value proposition, free/open-source model, security posture, and donation-supported sustainability, then choose the right product entry point.

**Why this priority**: Product-market fit depends on clear messaging, not just technical integration.

**Independent Test**: Run first-contact journeys for new evaluators and confirm they can identify the suite promise, map each app to a job-to-be-done, and find support/donation channels without confusion.

**Acceptance Scenarios**:

1. **Given** the suite home page, **When** a new visitor lands, **Then** they see a clear positioning line and the four message pillars before deeper navigation.
2. **Given** product detail pages, **When** a visitor compares apps, **Then** they can distinguish each app’s role and how cross-app workflows connect them.
3. **Given** the support page, **When** a user wants to back the project, **Then** donation options are visible, trusted, and non-blocking.

---

### User Story 6 - Use major identity providers with groups out of the box (Priority: P1)

A platform admin can configure major identity providers (starting with Entra ID) and map group claims to suite roles/scopes, using published provider guides and a consistent provider model across all products.

**Why this priority**: Shared auth and group support are required for secure multi-team operations and enterprise adoption.

**Independent Test**: Configure two different providers in a staging environment, map groups to team roles, sign in users from each provider, and verify identical authorization outcomes for the same group policy.

**Acceptance Scenarios**:

1. **Given** an enabled provider (for example Entra ID), **When** an admin configures issuer/client/scopes/claims, **Then** users can sign in and receive role/group-based access according to suite policy.
2. **Given** a provider with group overage or missing group claims, **When** a user signs in, **Then** the suite shows a clear remediation path and refuses unsafe implicit access.
3. **Given** official provider docs are updated, **When** the suite updates its provider guide set, **Then** provider-specific setup guidance remains versioned and auditable.

---

### User Story 7 - Run any product alone, join the suite when wanted (Priority: P1)

An operator can install and run Fathom, Depths, Keel, or Ballast on its own, with no portal, and later enroll it in a suite to share policy, audit, and workflows. Leaving the suite returns the product to standalone without losing its own data.

**Why this priority**: Each product must be adoptable on its own; the suite adds value on top and must never become a single point of failure for the products it connects.

**Independent Test**: Run a product with no portal, no event transport, and no shared database configured and pass its full test suite. Then enroll it, confirm suite bindings apply, make the portal unreachable past the bundle max age, and confirm local administrators keep full control while suite-origin grants expire. Then leave the suite and confirm local bindings and audit history are intact.

**Acceptance Scenarios**:

1. **Given** a product with no suite enrollment, **When** it starts and serves users, **Then** sign-in, authorization, approvals, and audit all work without contacting any portal.
2. **Given** an enrolled product, **When** the portal is unreachable for less than the bundle max age, **Then** the product keeps enforcing the last verified suite bundle with no change in access.
3. **Given** an enrolled product, **When** the portal is unreachable for longer than the bundle max age, **Then** suite-origin grants stop granting access, local-origin grants and suite suspensions keep applying, and administrators see that suite policy is stale.
4. **Given** a binding with suite origin, **When** a product administrator tries to edit it in the product, **Then** the edit is refused with a pointer to the portal.
5. **Given** an administrator leaving the suite, **When** they preview the change, **Then** they see every identity that would lose access and can convert chosen suite bindings to local bindings before confirming.

### Edge Cases

- A user has valid access in one app but revoked access in another product scope.
- A long-running workflow spans maintenance windows and resumes later.
- An external system emits out-of-order or duplicate events for the same operation.
- Two admins submit conflicting policy updates at nearly the same time.
- A workflow depends on an environment that becomes unreachable mid-run.
- A rollback step succeeds in one app and fails in another.
- Cross-app audit storage is temporarily degraded while workflow execution continues.
- Marketing tone is interpreted as playful while an operator is performing a high-risk action.
- Product pages drift and describe overlapping or contradictory app responsibilities.
- Donation prompts become intrusive and are mistaken for access gating.
- Provider claim formats differ across IdPs (string vs array, group IDs vs names).
- Users belong to too many groups and the provider emits overage indicators instead of full group lists.
- Group rename/deletion at the provider side breaks existing role bindings until remapped.
- The portal is down while a revocation is pending: the revocation still takes effect in every product no later than the bundle max age.
- A local binding grants what a suite binding does not: both apply (grants are a union); only a suite suspension overrides local grants.
- A product signs in with a provider that has no stable issuer or group claims (GitHub, Discord): suite group mappings do not apply to those identities; only explicit per-identity suite bindings do.
- A product on an older release receives a bundle with grant kinds it does not understand: it ignores those grants (grants nothing for them) and reports the version gap to administrators.
- A workflow step asks Depths for an action that requires passkey step-up (spec 054): the step pauses until the actor confirms in Depths; the portal cannot satisfy or skip step-up.
- A product is enrolled in a portal, then restored from a backup taken before enrollment: it comes up standalone and must be re-enrolled.

## Standalone and Suite Mode *(mandatory)*

Every product (Fathom, Depths, Keel, Ballast) is complete on its own. Suite mode is an opt-in enrollment, per product, that adds shared policy, audit aggregation, and cross-app workflows on top. It never replaces a product's own sign-in, authorization decisions, approval gates, or audit log.

### Modes

- **Standalone (default)**: the product signs users in as its own OIDC/OAuth client, decides authorization from its own store, and keeps its own audit log. It needs no portal, no event transport, and no shared database.
- **Suite mode (opt-in)**: a product owner or admin enrolls the product with a portal. The product still decides every request locally; it additionally receives signed policy bundles, forwards audit events, and accepts cross-app workflow steps through its public API.

### How the two layers combine

| Concern | Standalone | Suite mode |
|---|---|---|
| Sign-in | Product's own client with the organization's IdP | Unchanged. Single sign-on comes from products sharing one IdP; the portal is not a required broker |
| Session | Product's own session on its own origin | Unchanged. Products never share session cookies or an origin |
| Authorization decision | Local, from local bindings | Local, from local bindings plus the last verified suite bundle. The portal is never called per request |
| Bindings | All `local` | Each binding records origin `local` or `suite`. Suite bindings are read-only in the product. Effective grants are the union; suite suspensions override local grants |
| Revocation | Local, immediate | Pushed by the portal and applied on receipt; the bundle max age bounds the delay when a push is lost |
| Portal unreachable | n/a | Within max age: last bundle applies. Past max age: suite-origin grants expire, local grants and suspensions keep applying, administrators see a stale-policy notice |
| Audit | Product's own immutable log | Product log stays the source of truth; events are forwarded to the portal, buffered locally during outages, deduplicated by event id |
| Approvals and step-up | Product's own gates | Product gates still apply to workflow steps; portal approvals add to them and never replace them |
| Cross-app workflows | Not available | Portal calls each product's public API with per-step scoped credentials carrying the initiating actor |

### Identity and scope

- A person is identified by the issuer and subject of the IdP that signed them in. Suite bindings apply only to identities from issuers the enrolled product trusts.
- Each enrolled product publishes a scope mapping from suite scopes to its native scopes: Fathom cluster/namespace, Depths host/stack, Keel workspace/project, Ballast namespace. A suite scope with no mapping grants nothing in that product.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: Portal MUST refuse anonymous access to all portal resources. Each product continues to refuse anonymous access on its own; single sign-on across products comes from a shared IdP, not from the portal.
- **FR-002**: Portal MUST distribute shared role, team, and environment/namespace scope policy to enrolled products as signed policy bundles. Enforcement and the authorization decision remain in each product.
- **FR-003**: Portal MUST provide centralized management of suite-origin users, teams, role bindings, and approval policies. It MUST NOT edit a product's local-origin bindings.
- **FR-004**: Portal MUST publish policy updates and revocations to enrolled products immediately. Enrolled products MUST apply them on receipt and invalidate affected sessions; the bundle max age is the upper bound when a push is lost.
- **FR-005**: Portal MUST provide an auditable catalog of cross-app workflow templates with explicit prerequisites and allowed operators.
- **FR-006**: Portal MUST execute workflow steps with deterministic ordering, status tracking, and actor attribution across participating products.
- **FR-007**: Portal MUST support rollback policies for multi-step workflows and record rollback outcomes for each step.
- **FR-008**: Portal MUST prevent duplicate effective execution when identical workflow requests are retried or replayed.
- **FR-009**: Portal MUST preserve immutable, searchable audit history for authorization decisions, workflow execution, approvals, rollbacks, and policy changes.
- **FR-010**: Portal MUST support scoped delegation where one product can request actions in another product only through approved policy channels.
- **FR-011**: Portal MUST support product-level and workflow-level approval gates for sensitive actions.
- **FR-012**: Portal MUST expose real-time workflow health and final outcome summaries suitable for operators and compliance reviewers.
- **FR-013**: Portal MUST maintain shared branding primitives, navigation vocabulary, and product identity metadata across suite entry points.
- **FR-014**: Portal MUST include suite onboarding that explains each product role and common cross-app workflow paths.
- **FR-015**: Portal MUST support tenant-safe data and control-plane boundaries so one team cannot read or trigger another team’s protected operations.
- **FR-016**: Suite positioning MUST clearly communicate that core orchestration and management capabilities are free and open source.
- **FR-017**: Donation prompts MUST be encouraged and visible across suite entry points without blocking access to core product capabilities.
- **FR-018**: The portal MUST provide a centralized support/funding surface that links to approved donation channels and funding policy.
- **FR-019**: Suite brand architecture MUST be explicit: master brand `UwU Squad`, product brands `Fathom`, `Depths`, `Keel`, and `Ballast`.
- **FR-020**: Product UI surfaces MUST lead with product names, while suite marketing surfaces MUST present “UwU Squad Suite” as the umbrella.
- **FR-021**: The suite MUST use a consistent positioning line and subline across homepage, product pages, and onboarding.
- **FR-022**: The suite MUST maintain four message pillars across major marketing surfaces: open/free core, operator-grade security, cross-app workflows, and self-host ownership.
- **FR-023**: Tone rules MUST be enforced: friendly/playful voice allowed on marketing surfaces, direct/sober voice required for in-product operational and security-critical UX.
- **FR-024**: The suite MUST maintain one shared visual system (design tokens, icon grammar, state semantics) across all product entry points.
- **FR-025**: The suite MUST preserve distinct product marks and metadata so each app remains clearly identifiable within the umbrella brand.
- **FR-026**: `uwusquad.dev` information architecture MUST expose clear suite and product routes including home, product pages, workflows, and support surfaces.
- **FR-027**: The homepage MUST include a concise suite value statement and an app-comparison view that helps users choose the right product quickly.
- **FR-028**: Product pages MUST describe jobs-to-be-done, security posture, and cross-app integration paths without contradicting other product pages.
- **FR-029**: The suite MUST define a shared provider model for identity integration that all four products consume consistently.
- **FR-030**: The suite MUST support Entra ID and other major OIDC-capable providers through documented provider configurations and claim-mapping rules.
- **FR-031**: Group claims from providers MUST map to suite teams/roles/scopes through explicit, auditable bindings.
- **FR-032**: Provider onboarding docs MUST be maintained as first-class artifacts and linked from suite/operator documentation.
- **FR-033**: Provider-specific failure modes (missing claims, overage, invalid issuer, audience mismatch) MUST fail closed with actionable operator guidance.
- **FR-034**: The suite MUST support “docs-first provider support” where provider integrations can be enabled via standards-compliant OIDC setup without custom code forks.

#### Standalone and suite mode

- **FR-035**: Every product MUST run fully standalone by default, with no portal, no event transport, and no shared database configured, and MUST pass its full test suite in that configuration.
- **FR-036**: Suite membership MUST be an explicit, per-product opt-in performed by a product owner or admin, audited in both the product and the portal, and preceded by a preview of what suite bindings would grant.
- **FR-037**: Enrolled products MUST make every authorization decision locally from local bindings plus the last verified suite bundle, and MUST NOT call the portal on the request path.
- **FR-038**: Policy bundles MUST be versioned and signed with a key established at enrollment. Products MUST reject unsigned bundles, bundles with a bad signature, and bundles older than the one they hold.
- **FR-039**: Every binding MUST record its origin (`local` or `suite`). Suite-origin bindings MUST be read-only in the product. Effective grants MUST be the union of local and suite grants, except that a suite identity suspension MUST override local grants in every enrolled product until a newer bundle lifts it.
- **FR-040**: Each bundle MUST carry a max age (default 15 minutes, operator-configurable). When a product's bundle is older than its max age, suite-origin grants MUST stop granting access, local-origin grants and suspensions MUST keep applying, and administrators MUST see a stale-policy notice.
- **FR-041**: Suite bindings MUST identify people by IdP issuer and subject and apply only to identities from issuers the enrolled product trusts. Group-claim mappings MUST NOT apply to identities from providers without stable group claims; those identities MAY receive explicit per-identity suite bindings.
- **FR-042**: Each enrolled product MUST publish an explicit mapping from suite scopes to its native scopes. A suite scope without a mapping MUST grant nothing in that product.
- **FR-043**: Each product's own audit log MUST remain the source of truth. In suite mode, products MUST forward audit events to the portal with at-least-once delivery, buffer them locally while the portal is unreachable, and the portal MUST deduplicate them by event id.
- **FR-044**: Cross-app workflow steps MUST invoke products only through their public APIs, using credentials scoped to that step's action and target and carrying the initiating actor. Products MUST NOT expose endpoints reachable only by the portal.
- **FR-045**: Product approval gates and step-up requirements (including Depths passkey step-up, spec 054) MUST apply to workflow-invoked steps. A step that needs step-up MUST pause until the actor confirms in that product; the portal MUST NOT satisfy or bypass it.
- **FR-046**: Leaving the suite MUST show every identity that would lose access, MUST let an administrator convert chosen suite bindings to local bindings first, and MUST then remove suite bindings and stop forwarding while leaving local bindings and local audit history untouched.
- **FR-047**: The sync contract between portal and products MUST be versioned and negotiated at enrollment. A product MUST ignore bundle grant kinds it does not understand (granting nothing for them) and report the version gap.
- **FR-048**: Each product MUST run on its own origin and keep its own session cookies. `uwusquad.dev/<product>` routes are product information pages, not product runtime origins.
- **FR-049**: An event transport (for example NATS JetStream) MAY be required by the portal and by enrolled products for sync, but MUST NOT be required by a standalone product.

### Key Entities *(include if feature involves data)*

- **Suite User**: Identity with role assignments, team memberships, and access scopes.
- **Team Policy Binding**: Mapping of team roles to product scopes, namespaces/environments, and sensitive-action permissions.
- **Workflow Template**: Approved multi-product operation definition with ordered steps, prerequisites, and rollback policy.
- **Workflow Run**: Live execution record for a template including actor, status transitions, per-step outputs, and final outcome.
- **Approval Gate**: Policy checkpoint requiring one or more authorized approvers before execution can proceed.
- **Delegation Grant**: Explicit policy allowing one product context to invoke controlled actions in another product context.
- **Audit Event**: Immutable event record for authentication, authorization, policy, workflow, and rollback actions.
- **Product Identity Metadata**: Canonical suite metadata for product names, marks, icons, and navigation labels.
- **Brand Architecture**: Canonical mapping of suite brand, product brands, naming hierarchy, and usage rules by surface.
- **Messaging Pillar Set**: Versioned set of suite-level value pillars used across homepage, docs, onboarding, and campaigns.
- **Route Surface Map**: Canonical `uwusquad.dev` route taxonomy for suite home, product pages, workflows, and support/donation.
- **Auth Provider Profile**: Configurable provider definition including issuer metadata, client identifiers, scopes, and claim-mapping rules.
- **Group Claim Mapping**: Explicit transformation from provider group claims to suite team/role bindings.
- **Provider Docs Registry**: Versioned references to official provider setup docs and suite-specific setup guides.
- **Suite Enrollment**: A product's opt-in membership in a portal: product identity, bundle-signing trust, negotiated contract version, enrollment and leave history.
- **Policy Bundle**: Signed, versioned, max-aged set of suite bindings, suspensions, and approval policies distributed to enrolled products.
- **Binding Origin**: Marker on every role binding (`local` or `suite`) that decides who may edit it and what survives leaving the suite.
- **Identity Suspension**: Suite-issued block on an identity that overrides local grants in every enrolled product.
- **Scope Mapping**: Per-product translation from suite scopes to native product scopes.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: 100% of suite requests are authenticated and evaluated against shared policy before execution.
- **SC-002**: 100% of denied out-of-scope actions are blocked and auditable within one minute of request time.
- **SC-003**: At least 95% of approved cross-app workflows complete without manual operator intervention.
- **SC-004**: At least 90% of workflow failures with defined rollback policy end in a stable post-rollback state.
- **SC-005**: 100% of policy, approval, workflow, and rollback events are searchable by actor, team, product, and time range.
- **SC-006**: At least 85% of first-time operators complete one cross-app provisioning workflow in under 20 minutes.
- **SC-007**: 100% of core orchestration workflows remain usable without donation or paid enrollment.
- **SC-008**: At least 20% of monthly active administrators view a donation/support surface during normal usage.
- **SC-009**: At least 85% of first-time evaluators can correctly map all four products to their intended use case after viewing suite onboarding.
- **SC-010**: At least 90% of sampled suite surfaces match the approved positioning line, pillar language, and tone rules in quarterly content audits.
- **SC-011**: At least 80% of evaluators can choose an initial product path within five minutes on first visit.
- **SC-012**: At least 95% of supported-provider sign-ins resolve group-based authorization without manual post-login correction.
- **SC-013**: 100% of provider onboarding flows have an official docs reference and a suite-maintained setup guide.
- **SC-014**: All four products pass their full test suites with no portal, no event transport, and no shared database configured.
- **SC-015**: With the portal unreachable, enrolled products keep 100% of local-origin access working, and suite-origin grants stop granting access within the bundle max age plus one minute.
- **SC-016**: With the portal reachable, revocations and suspensions take effect in every enrolled product within one minute.
- **SC-017**: Leaving the suite preserves 100% of local bindings and local audit history.

## Assumptions

- `uwusquad.dev` is the primary suite domain and can host shared sign-in and portal experiences.
- Fathom, Depths, Keel, and Ballast remain standalone products with their own interfaces, sign-in, and authorization; shared identity and policy controls apply only to products an operator enrolls in the suite.
- Desktop Fathom (Tauri) runs standalone; it may enroll like any other product but never runs behind the portal.
- Existing team and namespace/project concepts remain the baseline for cross-app authorization scopes.
- Workflows are initially curated from a controlled catalog rather than arbitrary user-authored automation.
- Organizations adopting the suite accept mandatory auditability and explicit approval controls for sensitive actions.
- Donation encouragement remains sustainability-focused and does not convert core capability into paid access.
- Providers are integrated primarily via standards-based OIDC and claim mapping, with provider-specific adapters added only when standards behavior is insufficient.
