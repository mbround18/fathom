# Feature Specification: Optional In-Cluster Daemon (Controller Events & Durable Watches)

**Feature Branch**: `036-incluster-daemon`

**Created**: 2026-09-24

**Status**: Draft

**Input**: docs/MVP.md §14 Tier D P4 — "optional lightweight in-cluster daemon (reusing the existing Rust backend/protocol crates rather than a new stack) that can push richer or lower-latency context than polling the API server from outside the cluster — e.g. controller-level events, long-lived watches that survive the desktop client disconnecting/reconnecting. Strictly optional — Fathom already works with zero in-cluster footprint today."

## Relationship to other specs (overlap boundaries)

- **012 (P1 shipped)**: multi-context sidebar. This feature adds one more per-context status facet (daemon state); it does not touch context switching.
- **027 fleet rollup**: polls each context from outside. The daemon may later feed it; out of scope here, fleet behavior is unchanged.
- **031 hosted server mode**: a per-user, OIDC-fronted web app that is the UI's backend. The daemon is different: a headless, shared, read-only observer with no UI, no login and no per-user sessions. Both may ship in the same container image and reuse `deploy/Dockerfile` and `k8s/cluster.sh`, but neither depends on the other at runtime. Sequencing: `k8s/` and `deploy/` land with 031; tasks touching them are marked as depending on 031 being merged.
- **035 cloud auth plugins**: unrelated; the desktop reaches the daemon with whatever credentials the context already uses.

## Clarifications

### Session 2026-09-24

- Q: When several teams or users share one cluster, how should the daemon decide which namespaces' events, metrics and topology each user can see? → A: Filter per user. The daemon authorizes every request as the calling user (verified identity forwarded by the API server via an aggregated API / `APIService` registration; clients never send tokens to the daemon), checks it with `SubjectAccessReview` per namespace (briefly cached), returns only what that user can list, and fails closed.
- Q: Should the daemon also keep a history of metrics and a live topology graph, or stay events-only? → A: Events plus a live topology graph derived from its existing watches and a short bounded in-memory metrics ring (~1 h, a few series per object). Durable time-series storage is a separate later spec behind an adapter interface; not in 036.
- Q: What licence bar should a storage backend meet before Fathom bundles it, or lists it as supported, in the follow-on spec? → A: Anything bundled or shipped as a sidecar MUST be permissively licensed (Apache-2.0, MIT, BSD). Adapters to external or remote backends are allowed under any licence because Fathom only talks to them over a network protocol. Licence status of each candidate (GreptimeDB, InfluxDB, CnosDB, QuestDB, remote) is verified in the follow-on spec's research, not assumed.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Catch what happened while I was away (Priority: P1)

An operator has installed the daemon in a cluster. They close the laptop for an hour; a Deployment rolls out, a CronJob fails twice, an HPA scales up. When they reopen Fathom and connect to that context, the daemon replays a bounded backlog of controller-level events from the gap and the operator sees a "While you were away" timeline instead of only current state.

**Why this priority**: Core value over direct API access: history and continuity across client disconnects.

**Independent Test**: With the daemon on kind, trigger a rollout and a failing Job, connect Fathom afterwards, confirm both appear with correct timestamps; disconnect, trigger more changes, reconnect, confirm only the missed events arrive (no duplicates, no loss within retention).

**Acceptance Scenarios**:

1. **Given** the daemon is running and the client was disconnected, **When** the client reconnects with its last-seen cursor, **Then** it receives exactly the events after that cursor, in order.
2. **Given** the gap exceeds retention (or the daemon restarted), **When** the client reconnects, **Then** it is told explicitly that history was truncated (with the earliest available time) and performs a normal full sync; it never shows a partial timeline as complete.
3. **Given** a controller-level change (rollout progress/completion, condition transitions such as Available/Progressing/Failed, HPA scaling decisions, Job/CronJob success or failure), **When** it occurs, **Then** the daemon emits one derived event naming the object by namespace/name with reason and message, alongside the raw Kubernetes Event when one exists.
4. **Given** the timeline, **When** the operator views an entry, **Then** the reason/message is displayed verbatim.

---

### User Story 2 - Fathom works exactly as before with no daemon (Priority: P1)

An operator never installs the daemon (or installs then removes it). Fathom behaves identically to today: no errors, no prompts, no delay, no requirement.

**Why this priority**: A hard roadmap constraint; a regression here breaks every existing user.

**Independent Test**: Run existing quickstarts/e2e against a cluster with no daemon and confirm no behavior change and no daemon error UI; stop the daemon mid-session and confirm graceful fallback.

**Acceptance Scenarios**:

1. **Given** a context with no daemon, **When** the operator connects, **Then** connection, views and latency are unchanged; the only trace is a neutral "Daemon: not installed" line in context details.
2. **Given** the per-context daemon preference is off (default), **When** Fathom connects, **Then** it makes zero requests to any daemon endpoint.
3. **Given** a daemon becomes unreachable mid-session, **When** the stream drops, **Then** Fathom keeps operating from direct API access, marks the daemon "unavailable" with the cause inspectable, and retries with bounded backoff.
4. **Given** the daemon is absent and the preference is on, **When** Fathom probes, **Then** the probe is one bounded request (5 s timeout), runs only on connect, manual refresh or backoff retry, and never blocks connection.

---

### User Story 3 - Deploy and remove the daemon with one command; test on kind (Priority: P2)

An operator deploys the daemon with `kubectl apply -k` on the repo's `./k8s` overlays, verifies it, and removes it cleanly. A developer brings up a local kind cluster with the daemon, sample workloads that generate controller events, and runs the automated integration test.

**Why this priority**: No adoption or verification path without it, but it only delivers value after Story 1's daemon exists.

**Independent Test**: On a clean kind cluster run the documented commands, confirm the daemon is Ready and Story 1's scenario passes; run uninstall and confirm nothing (RBAC, namespace, CRDs) remains.

**Acceptance Scenarios**:

1. **Given** a kind cluster, **When** the operator runs the documented kustomize install, **Then** a single-replica Deployment in its own namespace becomes Ready with a read-only, least-privilege service account.
2. **Given** the overlay, **When** inspected, **Then** it grants no write verbs, no Secrets access, and adds no CRDs, webhooks or mutating behavior.
3. **Given** the install, **When** removed via the documented command, **Then** every namespaced and cluster-scoped object it created (including the `APIService` registration) is gone.
4. **Given** the kind harness, **When** run, **Then** it deploys the daemon image, generates sample controller events, and asserts replay, truncation and reconnect behavior.

---

### User Story 4 - Warm long-lived watches feed live views cheaply (Priority: P3)

When the daemon is present and enabled, high-churn lists (Pods, Events) can be sourced from the daemon's warm watch cache so connecting to a large cluster shows a populated list without the client opening its own full LIST+WATCH.

**Why this priority**: A performance enhancement on top of Stories 1-2; the timeline is valuable without it.

**Independent Test**: Against a cluster with thousands of pods compare time-to-first-populated-list with and without the daemon source; never worse than direct access, and list content matches direct-API state.

**Acceptance Scenarios**:

1. **Given** the daemon holds a warm cache for a kind, **When** the client requests a snapshot, **Then** it receives snapshot plus subsequent deltas consistent with the API server.
2. **Given** the cache is stale beyond a threshold or unhealthy, **When** the client requests a snapshot, **Then** it transparently falls back to direct API access.
3. **Given** a superseded context or request, **When** late daemon events arrive, **Then** they are discarded.

---

### User Story 5 - Near-real-time topology and short metrics history from the cluster (Priority: P3)

With the daemon enabled, the operator opens a topology view that shows workload, pod, node, service and ownership relationships updating within seconds, and sees recent trends (about the last hour) for a few key series per object, without Fathom having run its own watches while it was closed.

**Why this priority**: Builds on the daemon's existing watches at little extra cost, but is only useful once Stories 1-2 exist.

**Independent Test**: On kind, scale a Deployment and cordon a node; the topology reflects both within 5 s. Reconnect after 30 minutes and confirm the trend ring covers the gap. Two users with different namespace access see different graphs from the same daemon.

**Acceptance Scenarios**:

1. **Given** the daemon is running, **When** an object or ownership relation changes, **Then** subscribed clients receive a topology delta within 5 s of the API server change.
2. **Given** a user with access to only some namespaces, **When** they request topology or trends, **Then** nodes and edges outside their access are absent (FR-017), and cross-namespace edges to hidden objects are dropped, not shown as placeholders.
3. **Given** the metrics ring, **When** it reaches its bound, **Then** the oldest samples drop with an explicit truncation marker; trends are never shown as complete when truncated.
4. **Given** a future durable store is configured (out of scope here), **Then** the daemon's ring and topology APIs are unchanged; storage sits behind an adapter interface.

---

### Edge Cases

- Daemon restart: new epoch; clients see a truncation marker and resync.
- Two clients connected at once: each has its own cursor; one shared buffer serves both.
- Client lacks permission to reach the daemon's aggregated API group: shown as "no access", distinct from "not installed"; direct path continues.
- Daemon/client version skew: capability handshake; unknown event kinds ignored; incompatible major means "unavailable: version mismatch", never a crash.
- Very chatty cluster: buffer bounded, oldest drop with an explicit truncation count; per-kind rate limit on derived events with a coalesced "N more" marker.
- Daemon pod evicted/OOM: Kubernetes restarts it; buffer loss surfaces as truncation.
- Namespaces outside the daemon's scope are absent; the UI states the daemon's scope.
- Two users with different RBAC on the same daemon each get only their own namespaces from the same shared buffer; a user whose permissions change mid-session sees the change within the 60 s cache window.
- The cluster cannot register an `APIService` (managed clusters that restrict it) or the API server sends no verified identity: the daemon returns nothing and the client shows `unavailable` with the cause; it never falls back to unfiltered output and never accepts a client-supplied token.
- Clock skew: ordering uses daemon-assigned sequence numbers, not client time.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: Provide a separate, optional daemon binary reusing the existing Rust backend and protocol crates (no new stack); the desktop app MUST NOT require it.
- **FR-002**: With no daemon installed, every existing behavior MUST remain unchanged (zero in-cluster footprint).
- **FR-003**: The daemon MUST keep long-lived watches on Events and on Deployments, StatefulSets, DaemonSets, ReplicaSets, Jobs, CronJobs, HPAs and Pods, and derive controller-level events from condition/status transitions.
- **FR-004**: The daemon MUST retain a bounded, in-memory, sequence-numbered event log and serve replay-from-cursor, with an explicit truncation marker when the cursor is outside retention or the epoch changed.
- **FR-005**: The daemon MUST be read-only (get/list/watch), MUST NOT read Secrets, and MUST NOT expose payloads containing secret data.
- **FR-006**: Clients MUST reach the daemon through the API server as an aggregated API (a registered `APIService`) using the context's existing credentials, so the API server authenticates the user, access is gated by cluster RBAC, and no new listener, ingress, login or credential is introduced. Clients MUST NOT send their token or any credential to the daemon.
- **FR-017**: The daemon MUST authorize every request as the calling user: it accepts identity only from the API server's verified front-proxy headers (client-certificate authenticated), checks list access per namespace with `SubjectAccessReview` (cached for at most 60 s), and returns only objects, events and derived data in namespaces that user can list. If the identity is missing, unverified or a check errors, it MUST return nothing for that scope (fail closed) and report `noAccess` or the check error verbatim, never an unfiltered view. Cluster-scoped kinds require the user's cluster-level list access. Filtering MUST apply to replay, live streams, topology and metrics alike.
- **FR-018**: The daemon MUST derive a live topology graph (owner references, selectors, pod-to-node, service-to-pod) from its existing watches, serve snapshot plus deltas with sequence numbers, and filter it per user (FR-017). It MUST NOT add watches beyond FR-003 plus Nodes and Services.
- **FR-019**: The daemon MUST keep a bounded in-memory metrics ring (default about 1 h, a small fixed set of series per object, sourced from the metrics API when present) with explicit truncation markers; when the metrics API is absent it reports `metricsUnavailable` rather than empty data.
- **FR-020**: Metrics and topology retention/storage MUST sit behind an internal storage interface so a durable time-series backend can be added later without protocol changes; no third-party database is bundled by 036. When a follow-on spec adds durable storage, bundled/sidecar backends MUST be permissively licensed (Apache-2.0, MIT, BSD) and external/remote adapters may use any licence; the licence of each candidate is verified from source, not assumed.
- **FR-007**: The client MUST detect the daemon via a bounded, opt-in probe and expose per-context daemon state (`notInstalled`, `available`, `unavailable`, `noAccess`, `versionMismatch`) in the protocol and UI.
- **FR-008**: The client MUST fall back to direct API access automatically whenever the daemon is absent, stale or failing, with inspectable status.
- **FR-009**: Protocol changes MUST be additive (new events/commands defined in Rust, bindings generated, round-trip tests) with a capability handshake for the daemon API.
- **FR-010**: The UI MUST show a "While you were away" timeline of daemon-derived events, reasons/messages verbatim, distinguishing derived from raw Events.
- **FR-011**: Fathom MUST NOT install, upgrade or remove the daemon; installation is an explicit operator action (Principle I). The UI MAY show the documented install command.
- **FR-012**: The daemon MUST be deployable and removable via `./k8s` kustomize overlays, ship as a container image from the shared Dockerfile, and be covered by an automated kind integration test.
- **FR-013**: Resource use MUST be bounded: configurable buffer size and retention; target under 250 MiB RSS (including topology and the metrics ring) and under 2% of one core idle at 5,000 objects; single replica by default.
- **FR-014**: The daemon MUST expose health/readiness and version/capability endpoints and log without secrets.
- **FR-015**: Client MUST discard events from a superseded context/session; high-volume streams MUST be batched and bounded.
- **FR-016**: Existing `~/.fathom/` state MUST load unchanged; the per-context daemon preference is additive.

### Key Entities

See [data-model.md](./data-model.md).

## Success Criteria *(mandatory)*

- **SC-001**: After a disconnect, reconnect delivers all missed events within retention in under 2 s for 1,000 buffered events, with zero duplicates or gaps (kind test).
- **SC-002**: With the daemon absent, connection time and first-populated-list time are within noise of the pre-feature baseline, and zero daemon requests occur when the preference is off.
- **SC-003**: The kind install/uninstall round trip completes with all created objects removed and Story 1 passing, from a documented command sequence.
- **SC-004**: The daemon stays within FR-013 budgets in the 5,000-object kind fixture.
- **SC-005**: Killing the daemon mid-session never interrupts direct-API views; status flips to `unavailable` within 10 s.
- **SC-007**: A topology change appears in a connected client within 5 s at the 5,000-object fixture, and per-user filtering hides every out-of-scope node and edge (kind test with two users).
- **SC-006**: Every new protocol variant has a round-trip test and the binding drift check passes in CI.

## Assumptions

- Defaults chosen without clarification: transport is NDJSON HTTP streaming over the API server's aggregated-API path; retention 10,000 events or 24 h, whichever is smaller; probing is a per-context opt-in ("Use in-cluster daemon if present"), default off; cluster-wide read scope by default with an optional namespace allowlist.
- Reaching the daemon goes through an `APIService`-registered API group; what each user sees is filtered to their own RBAC (FR-017). Users with no list access in a namespace never see its events. Operators needing hosted per-user sessions can still use 031.
- The buffer is in-memory only (restart = truncation); persistence is a non-goal.
- Single replica; HA/leader election is a non-goal.
- Non-goals: a bundled or durable time-series database (deferred to a later spec with adapters), mutation via the daemon, fleet aggregation, cross-cluster federation, Fathom-managed install, public ingress.
