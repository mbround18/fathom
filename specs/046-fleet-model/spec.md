# Feature Specification: Fleet model

**Feature Branch**: `046-fleet-model`

**Created**: 2026-10-02

**Status**: Draft

**Input**: User description: "spec out the fleet model and cross-cluster search" — from the adversarial product review (2026-10-02), finding 1 of 5: *"you don't have a fleet model, you have a context list."* `FleetDashboardView.tsx:60` maps `session.contexts` straight to a grid of cards in kubeconfig order; `FleetContextSummary` (`src/protocol/v1/fleet.rs:45`) carries health, counts, version and Flux state and **no** identity beyond `context`; the card renders that raw string as its only label (`FleetContextCard.tsx:57`). There is nothing to name a cluster, nothing to group clusters by, and nothing for a cross-cluster operation to be scoped to. This feature adds the model. The first consumer, cross-cluster search, is `047-cross-cluster-search` and is specified separately so each lands on its own.

## Clarifications

### Session 2026-10-02

- Q: What identifies a fleet member? → A: The kubeconfig context name, exactly as every other per-cluster record in Fathom already does — `cluster-settings.json`, `federation.json`, exec policies and node-shell settings are all keyed that way (`src/backend/persistence.rs:567,998,502,656`). Not the API server URL: spec 012's edge case already settled that the same API server reached through two contexts is two independent entries with no de-duplication, and that stays true here. A member is therefore created by a context existing, not by an operator adding one; what this feature adds is what Fathom knows *about* it.
- Q: Why does a cluster need a display name when it already has a context name? → A: Because the context name is frequently not a name. A kubeconfig written by `aws eks update-kubeconfig` names the context `arn:aws:eks:us-east-1:123456789012:cluster/prod-blue`; GKE writes `gke_my-project_us-central1-a_prod`. The card truncates it in a monospace span and the operator reads an ARN fragment. A display name is a label over the key, never a replacement for it: both are shown wherever a member is identified, and the context name remains the only thing anything is keyed on.
- Q: How are clusters grouped? → A: Labels and label selectors, the idiom the operator already has. Each member carries a `key=value` label map (`env=prod`, `region=us-east-1`, `owner=payments`); a **group** is a *named label selector* over the fleet (`prod` = `env=prod`, `us` = `region in (us-east-1, us-west-2)`). Reason: Principle I says Fathom is for operators who already understand Kubernetes and that abstractions hiding Kubernetes objects are out of scope. Inventing a bespoke folder or tag tree would be exactly that invention; a selector is something they can already read, and it composes, so a cluster lands in every group it matches without being filed anywhere. It also hands `047` a scope expression for free.
- Q: Where does this persist? → A: A new `~/.fathom/fleet.json`, version-stamped, additive, unreadable-falls-back-to-defaults-without-deleting (Principle VI). **Not** `cluster-settings.json`: its own type carries the instruction "§10's field set, exactly — this feature MUST NOT add any other preference category here" (`src/protocol/v1/team.rs:57-58`), and fleet identity is not a metrics preference. **Not** `federation.json` either: that file records which contexts the operator chose to hold a live connection to, which is a session-shaped decision with a soft cap, while fleet membership is a durable description of what exists. The two are read together and stay separate files.
- Q: A platform team needs one definition of "prod". How is that shared? → A: Through the shared store `014-teams-organizations` already built, by the mechanism it already uses. Labels and group definitions publish and are adopted exactly as `ConnectionProfile` does (`src/protocol/v1/team.rs:137`): a monotonic `rev`, a `shared_by`, a `removed` flag rather than a deletion, and an adopted local copy that keeps working when the store goes away (`AdoptedConnectionProfile::still_shared`, ibid. 158). Per-field origin is reported the way cluster settings already report it (`ClusterSettingsFieldOrigin` / `SettingOrigin`, ibid. 44), so the operator can see that `env=prod` came from the team and `owner=me` is local. A local value always wins over a shared one and says so.
- Q: What happens to a member whose context has left the kubeconfig? → A: It is shown as unavailable with its last-known labels, and pruned only on an explicit operator action — 034's rule for a persisted federated context, applied here for the same reason (Principle VI: an operator's curation is their workflow, and a kubeconfig that is temporarily missing a context must not silently erase what was written about it).
- Q: Does this run in hosted mode? → A: The model does; the fleet view does not, and this feature does not change that. A hosted deployment serves exactly one cluster — `HostedDenial::Fleet` denies `StartFleetPolling`, `StopFleetPolling` and `RequestFederationSnapshot`, and `SetContext` is allowed only when the context is `home` (`src/protocol/v1/mutation.rs:1382,1402,1485`). So hosted gets a fleet of one: that cluster can carry a display name and labels, published through the same shared store, which is what lets a team see the same `env=prod` in hosted and on the desktop. Lifting the hosted fleet denial is a security-posture decision about whether a hosted server may hold N cluster credentials, it is **out of scope here**, and it is recorded as an open product question in the review rather than decided inside a data-model feature.
- Q: Does the fleet model do any background work? → A: None. It is read at launch and written when the operator edits it. It starts no pollers, holds no clients and watches nothing; Principle V's "an idle app MUST use under 2% of one CPU core" is untouched because nothing here runs when nobody is looking. The shared-store refresh rides on 014's existing refresh, which is already operator-triggered.
- Q: Is a group's selector allowed to match zero clusters? → A: Yes, and it says so. A group that matches nothing is a legitimate state — the operator defined `env=staging` before the staging context was in the kubeconfig — and it is shown as "matches no cluster" rather than hidden or rejected. Hiding it would assert that the group is wrong when the truth is that the fleet does not currently contain a match.
- Q: What does the dashboard do when one cluster's credentials cannot read what a group's health needs? → A: Exactly what it does today, which is already right: `FleetContextSummary.restricted` lists the resource groups the credentials could not read, the matching counts are `None`, and the card shows "no access" (`src/protocol/v1/fleet.rs:54-57`). A group rollup that spans a denied cluster is reported as a partial over N of M clusters and names the ones it could not see. It never presents a sum over a subset as a sum over the group. This is the same rule the review found broken elsewhere — `integrationVisible` (`ui/src/resources/nav.ts:64-69`) hides Velero when the scan was *denied*, asserting "not installed" where the truth is "I could not look" — and this feature must not repeat it.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Name and label the clusters I actually have (Priority: P1)

An operator with a dozen kubeconfig contexts, several of them EKS ARNs, opens the fleet view. They
give each cluster a readable display name and attach labels: `env=prod`, `region=us-east-1`,
`owner=payments`. The cards show the display name with the context name beneath it, and the fleet
can be sorted and filtered by label. The names and labels survive a restart.

**Why this priority**: It is the whole model in its smallest useful form, and on its own it fixes
the thing an operator hits first — a grid of truncated ARNs in kubeconfig order. Every later story
reads the labels this one writes.

**Independent Test**: With three contexts in the kubeconfig, set a display name and two labels on
each, restart Fathom, and confirm all nine values are still there and the cards are identified by
display name.

**Acceptance Scenarios**:

1. **Given** a context named `arn:aws:eks:us-east-1:123456789012:cluster/prod-blue`, **When** the operator sets its display name to `prod-blue`, **Then** the card is headed `prod-blue` and the full context name is still shown and still copyable.
2. **Given** labels `env=prod` on two contexts and `env=dev` on one, **When** the operator filters the fleet by `env=prod`, **Then** exactly the two cards remain and the count states "2 of 3 clusters".
3. **Given** a display name and labels were set, **When** Fathom restarts, **Then** they are restored before the first card renders health, and nothing was written to `cluster-settings.json`.
4. **Given** `~/.fathom/fleet.json` is corrupt, **When** Fathom starts, **Then** the fleet falls back to unnamed, unlabelled members, the operator is told the file could not be read with the verbatim reason, and the file is left on disk untouched.
5. **Given** a context is removed from the kubeconfig, **When** the fleet view opens, **Then** its member is listed as unavailable with its labels intact, and is removed only when the operator removes it.

---

### User Story 2 - Group clusters by selector and see each group's rollup (Priority: P2)

The operator defines groups as named label selectors — `prod` = `env=prod`, `us` =
`region in (us-east-1, us-west-2)` — and the fleet view organises itself by group, with a rollup
per group: clusters in it, how many are healthy, how many nodes ready of total, how many unhealthy
pods. A cluster appears in every group it matches. A rollup that could not see every member says
so and names the ones it missed.

**Why this priority**: This is what turns a list into a fleet, and it is the scope expression
`047` needs. It depends on US1's labels, and US1 is useful without it.

**Independent Test**: Label three contexts, define two overlapping groups, and confirm each group
shows the right members and a rollup whose numbers equal the sum of its members' cards.

**Acceptance Scenarios**:

1. **Given** groups `prod` (`env=prod`) and `us` (`region=us-east-1`) and a cluster labelled both, **When** the fleet view renders, **Then** that cluster appears under both groups and is counted once in each rollup.
2. **Given** a group whose selector matches no cluster, **Then** it is shown with "matches no cluster", not hidden.
3. **Given** a three-member group in which one cluster's node reads are denied, **Then** the rollup reports nodes over the two it could read, states "2 of 3 clusters", and names the third as not permitted — never a sum presented as complete.
4. **Given** a member becomes unreachable, **Then** its group's rollup is partial in the same way and the reachable members keep updating.
5. **Given** an invalid selector expression, **When** the operator saves it, **Then** it is rejected at the point of entry with the parse error, and no group is created.

---

### User Story 3 - Share one definition of "prod" with the team (Priority: P3)

A platform lead publishes the fleet's labels and group definitions to the shared store that
`014-teams-organizations` already configures. A teammate adopts them and sees the same groups
their colleague sees. Each value shows where it came from; a local edit overrides the shared value
and is marked as local. Losing access to the store never deletes what was adopted.

**Why this priority**: It is what makes the model a team artifact rather than one laptop's notes,
and it is the answer to the review's WHO finding. It is operational polish on top of US1/US2,
which work entirely locally.

**Independent Test**: Publish labels and two groups from one profile, adopt from a second with its
own local label on one cluster, and confirm the shared values arrive, the local one wins and is
marked local, and making the store unreachable changes nothing that was already adopted.

**Acceptance Scenarios**:

1. **Given** a shared label set at `rev` 4 and a local edit to one of its keys, **Then** the local value is used, is marked as local, and the shared value remains visible as the origin it overrides.
2. **Given** an adopted group, **When** the shared store becomes unreachable, **Then** the group keeps working and is marked as no longer following the shared copy, with the verbatim store error available.
3. **Given** an adopted group is removed upstream, **Then it is flagged "no longer shared" and kept** until the operator removes it.
4. **Given** a hosted deployment and a desktop sharing one store, **Then** the single hosted cluster shows the same display name and labels the desktop shows for it.

### Edge Cases

- Two contexts pointing at the same API server: two members, independently named and labelled, no de-duplication (012).
- A label key or value that is not a valid Kubernetes label (too long, illegal characters): rejected at entry with the reason, because `047` will pass these to an API server as a selector and a value that cannot survive that round trip must not be storable.
- The same display name on two members: allowed, since the context name is the key; the context name disambiguates them wherever both appear.
- A member with no labels at all: belongs to no group, and is reachable through an explicit "ungrouped" listing rather than disappearing from a group-organised view.
- 100 contexts in one kubeconfig: the model is metadata only and does not poll, so it must render and filter without approaching the pane budgets Principle V sets; the live/poll caps of 8 (`src/backend/fleet.rs:45`) are unchanged and unrelated.
- A group selector that matches every cluster: legal, and not special-cased.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: Fathom MUST model each kubeconfig context as a fleet member keyed by its context name, and MUST NOT key fleet metadata on the API server URL or de-duplicate two contexts that share one.
- **FR-002**: A member MUST be able to carry an operator-set display name, and every surface that identifies a member MUST show both the display name and the context name; the context name MUST remain copyable verbatim.
- **FR-003**: A member MUST be able to carry a map of `key=value` labels, validated at entry against Kubernetes label syntax, with the validation failure reported verbatim.
- **FR-004**: A group MUST be a named label selector over fleet members, supporting equality and set-based expressions, and a member MUST appear in every group whose selector it matches.
- **FR-005**: A group whose selector currently matches no member MUST be shown as matching nothing, and MUST NOT be hidden, rejected or deleted.
- **FR-006**: An invalid selector expression MUST be rejected when entered, with the parse error shown verbatim, and MUST NOT create a group.
- **FR-007**: Fleet metadata MUST persist in its own file under `~/.fathom/`, MUST carry a schema version, MUST change only additively, and MUST fall back to defaults without modifying or deleting the file when it cannot be read — reporting the verbatim reason.
- **FR-008**: Fleet metadata MUST NOT be written into `cluster-settings.json`, `federation.json`, or any existing per-context file.
- **FR-009**: A member whose context is absent from the current kubeconfig MUST be listed as unavailable with its metadata intact, and MUST be removed only by an explicit operator action.
- **FR-010**: A group rollup that could not read every member MUST state how many members it covers out of how many, and MUST name the members it could not read and why, using the API server's own reason. It MUST NOT present a partial aggregate as complete.
- **FR-011**: A member's inability to read a resource because of RBAC MUST be reported as not permitted, distinctly from the resource being absent and distinctly from the member being unreachable.
- **FR-012**: Labels and group definitions MUST be publishable to, and adoptable from, the shared store that `014-teams-organizations` configures, using that feature's existing revision, attribution and removal semantics.
- **FR-013**: Every shared value MUST report its origin, a local value MUST take precedence over a shared one and MUST be marked as local, and the overridden shared value MUST remain visible.
- **FR-014**: Loss of access to the shared store, or upstream removal of an adopted definition, MUST NOT delete the adopted local copy; it MUST be marked as no longer following the shared copy.
- **FR-015**: The fleet model MUST start no background work: no pollers, no clients, no watches. It is read at launch and on explicit refresh, and written on operator edit.
- **FR-016**: The fleet view MUST be able to filter and sort members by label and by group, and MUST state how many of how many members a filter is showing.
- **FR-017**: Members matching no group MUST remain reachable through an explicit ungrouped listing.
- **FR-018**: In hosted mode a fleet of exactly one member MUST be supported — display name, labels, and shared-store participation — while the fleet dashboard, fleet polling and context switching remain denied as they are today. No existing hosted denial may be relaxed by this feature.
- **FR-019**: Every new protocol command and event MUST be defined in Rust with generated TypeScript bindings, and MUST have a serialization round-trip test (Principle II, III).
- **FR-020**: Fleet metadata MUST NOT contain credentials, tokens, or any secret, and the persisted file MUST be inspectable by the operator without revealing any.

### Key Entities

- **Fleet member**: one kubeconfig context as the fleet knows it. Key: context name. Attributes: display name, label map, origin of each value, availability (present in kubeconfig / absent), and whichever shared revision it last followed.
- **Fleet group**: a named label selector over members. Attributes: name, selector expression, origin, and — computed, never stored — the members it currently matches.
- **Group rollup**: a group's aggregated health. Attributes: members covered, members total, the per-member reasons for any that were not covered, and the aggregates themselves. Always carries its own coverage.
- **Shared fleet definition**: the publishable form of labels and groups in the 014 store. Attributes mirror `ConnectionProfile`: id, `rev`, `shared_by`, `updated_at`, `removed`.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: An operator with 12 contexts, 5 of them ARNs or GKE-style names, can identify every cluster from its card without reading a context string.
- **SC-002**: Defining a group and seeing its members takes one selector expression and no per-cluster filing step; a newly labelled cluster joins every matching group with no further action.
- **SC-003**: Display names, labels and groups survive a restart and an upgrade with no manual step, and a corrupt metadata file never costs the operator their kubeconfig or their other settings.
- **SC-004**: Every aggregate the fleet view shows states its own coverage; no screen in the feature displays a sum over a subset without saying so.
- **SC-005**: Fleet metadata adds no measurable idle CPU and no additional API server requests over today's behaviour (Principle V).
- **SC-006**: A second operator adopting a published fleet definition sees the same group names and memberships as the publisher, and can still override one label locally without losing the shared value.
- **SC-007**: Filtering a 100-member fleet by label responds to input in under 100 ms (Principle V).

## Assumptions

- The operator's kubeconfig is the source of truth for which clusters exist; this feature describes them and never adds, removes or authenticates one.
- `014-teams-organizations`' shared store is the sharing mechanism; this feature adds payload to it and does not build a second one.
- Labels are operator-authored metadata about clusters. They are deliberately the same shape as Kubernetes labels so a selector is portable to `047`, but they are not read from, or written to, any cluster object.
- Cross-cluster search, compare and bulk action are consumers of this model and are specified separately (`047` and beyond). This feature ships without them and is useful without them.
- Whether a hosted server may ever hold more than one cluster's credentials is an open product question recorded in the 2026-10-02 review; this feature assumes today's answer, which is no.
