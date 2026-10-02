# Feature Specification: Cross-cluster search

**Feature Branch**: `047-cross-cluster-search`

**Created**: 2026-10-02

**Status**: Draft

**Depends on**: `046-fleet-model` (a search is scoped to a fleet group; without groups there is nothing to scope to)

**Input**: User description: "spec out the fleet model and cross-cluster search" — from the adversarial product review (2026-10-02), finding 2: a grep across `src` and `ui/src` for cross-cluster search, compare, diff or grouping returns nothing. There is no primitive that spans clusters. What exists is `ui/src/resources/search.ts`, a client-side case-insensitive **prefix** filter over the rows already loaded into one pane of the one active context — useful, and categorically not this. The review's judgement was that search is the cheapest and most convincing cross-cluster primitive, and the first thing an evaluator tries.

## Clarifications

### Session 2026-10-02

- Q: Can this be answered from state Fathom already holds? → A: No, and that is the central design constraint. The federation tier holds one persistent client and four bounded watches per federated context, but it **reduces every event to counters and keeps no objects** (`src/backend/federation.rs:3-5`; 034's own clarification says in terms that it "does not mean full per-cluster resource stores"). There is nothing in memory to search. So a search is a one-shot fan-out of real queries to real API servers, and the whole feature has to be honest about what that costs.
- Q: Keystroke or submit? → A: Submit, explicitly, and this differs from the in-pane filter on purpose. `compileQuery` runs per keystroke over rows already in memory and costs nothing. A fan-out across twelve clusters that fires per keystroke would issue hundreds of list requests against production API servers while someone types. The search box therefore runs on submit, shows what it is about to do, and is cancellable while it runs.
- Q: Prefix or substring? → A: Substring, and this also differs from the in-pane filter on purpose. In-pane prefix matching is right when the operator is looking at the list and narrowing it. Across a fleet the operator is looking for something half-remembered — a pod whose generated suffix they do not know, a release whose prefix they have forgotten — and prefix matching would make the common case fail. The divergence must be stated in the UI, because two search boxes with different semantics and no explanation is its own defect.
- Q: What can be searched, and what does each kind of query cost? → A: Four mechanisms, with honestly different costs, and the UI must show which one a query will use:
  1. **Label selector** — the API server does the work, server-side, one request per cluster per kind. Cheap.
  2. **Kind or CRD presence** — discovery only, one request per cluster. Cheapest.
  3. **Name or namespace substring** — requires listing the kind per cluster, then filtering locally. Cost is a list.
  4. **Container image** — Kubernetes has no field selector for image, so this lists pods and inspects `spec.containers[].image` and `initContainers`. It is the single most-wanted cross-cluster question ("where is this image running?") and the most expensive one. It is offered, and it is labelled.
- Q: Which kinds are searchable? → A: A declared, bounded set, not "everything". Fathom has ~50 single-context resource modules; fanning all of them across twelve clusters is not a feature, it is an outage. The searchable set is the kinds a fleet-level question is actually asked about — Pods, Deployments, StatefulSets, DaemonSets, Services, Ingresses, ConfigMaps, Secrets (names only), Nodes, Namespaces, Helm releases, CRDs — and the operator picks which of them a given search covers, defaulting to a small subset. Extending the set is an additive change, deliberately.
- Q: Secrets are in that list. Does search read secret data? → A: Never. Secrets are searchable **by name, namespace and labels only**. No secret value is read, matched, returned, logged or indexed, and a search term is never compared against secret data — Principle VI forbids it, and a cross-cluster index of secret values would be the worst artifact this product could produce. The same rule applies to ConfigMap values, which are searchable by key name and not by content.
- Q: What happens when one cluster in the scope denies the read? → A: It is reported as not permitted, with the API server's own reason, as one outcome among four per cluster: **matched** (n results), **no match**, **not permitted** (verbatim reason), **unreachable** (verbatim error). A result list that covered eight of twelve clusters says "8 of 12" and names the four and why. It never presents a partial sweep as a complete answer. This is the rule `046` FR-010 sets and the rule the review found broken in `integrationVisible` (`ui/src/resources/nav.ts:64-69`), where a denied CRD scan is rendered as "not installed".
- Q: How much concurrency, and what bounds the results? → A: Eight concurrent clusters, matching `MAX_CONCURRENT_POLLS` (`src/backend/fleet.rs:45`) and 034's federation soft cap, so a fleet-wide query cannot be a thundering herd. Results are bounded per cluster and in total, with an explicit truncation marker — the same discipline Principle V already requires of log views and the metrics ring, where a bounded buffer must say that it truncated rather than silently drop.
- Q: Does an abandoned search keep running? → A: No. Closing the results, changing the scope or starting another search cancels the in-flight fan-out, and results from a superseded search are discarded, never rendered — Principle V states that directly. A search is operator-triggered work with an operator-visible end, not background work.
- Q: What happens when the operator clicks a result in another cluster? → A: The active context switches to that cluster and the object opens, and the switch is announced rather than silent. This is the behaviour the fleet card already has (`FleetDashboardView` calls `selectContext` on click), and the alternative — rendering an object from a non-active context in a pane — would require the per-cluster resource stores that 034 deliberately refused. The results stay available after the switch so a sweep is not lost by following one of its rows.
- Q: Does this run in hosted mode? → A: Yes, with a scope of one, and it is a real feature there rather than a degenerate one. "Which namespace is this image running in" across a whole cluster is exactly as hard to answer in hosted as across a fleet, and the same fan-out answers it over namespaces instead of clusters. The query runs as the signed-in user against that user's RBAC (Principle VI: Fathom never escalates), so two users searching the same hosted server can legitimately get different coverage, and each is told their own. No hosted denial is relaxed; a scope of one needs none.
- Q: Does this feature mutate anything? → A: Nothing. It introduces no mutating command. A fleet-wide bulk action with a per-cluster manifest preview is the review's fifth recommendation and a later feature; doing it here would put a cross-cluster write behind a search box, which is precisely the kind of thing Principle I exists to prevent.
- Q: Why not build compare/diff in the same feature? → A: Because a sweep is useful alone and a diff is not useful without one. Cross-cluster compare — the same workload in two contexts, side by side through the diff view the Helm rollback flow already has — is the natural next feature and reuses this feature's fan-out. It is out of scope here so that this lands.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Find where something is running, across a group (Priority: P1)

An operator picks a fleet group — `prod` — types part of a name, and submits. Fathom queries each
cluster in the group concurrently and streams results in as they arrive: cluster, kind, namespace,
name, and why it matched. Clusters that are still being searched show as in progress; clusters that
denied the read or could not be reached are listed with the reason. The header states the coverage:
"24 results across 10 of 12 clusters". Clicking a result switches to that cluster and opens the
object.

**Why this priority**: It is the primitive. It is the first question anyone with more than one
cluster asks, it is unanswerable in Fathom today, and the review's judgement is that it alone
changes the sentence a reviewer writes about the product.

**Independent Test**: With three contexts labelled into one group, create a Deployment with a
shared name fragment in two of them, search the group for that fragment, and confirm both results
arrive with their cluster named, the third cluster reports no match, and the coverage line says
3 of 3.

**Acceptance Scenarios**:

1. **Given** a group of three reachable clusters and a matching object in two, **When** the operator searches for a name fragment, **Then** both results appear with their cluster, kind and namespace, and the coverage states 3 of 3 clusters.
2. **Given** one cluster in the group denies listing the searched kind, **Then** that cluster is listed as not permitted with the API server's verbatim reason, the other results still show, and the coverage states 2 of 3.
3. **Given** one cluster is unreachable, **Then** it is listed with its verbatim connection error and the search completes rather than hanging on it.
4. **Given** results are still streaming, **When** the operator cancels, **Then** the fan-out stops, no further requests are issued, and what arrived stays on screen marked as a cancelled partial.
5. **Given** a result in a non-active cluster, **When** the operator clicks it, **Then** the active context switches with the switch stated, the object opens, and the result list is still available.
6. **Given** a second search is submitted while the first is in flight, **Then** the first is cancelled and none of its late results are rendered.

---

### User Story 2 - Ask the expensive question on purpose: where is this image? (Priority: P2)

The operator searches a group by container image rather than by name. Fathom states before running
that this lists pods in every cluster in scope and inspects their containers, shows per-cluster
progress while it does, and returns every pod whose containers or init containers match, with the
matching image string shown.

**Why this priority**: It is the highest-value cross-cluster question and the one that justifies the
feature to a platform team — "is the vulnerable tag still running anywhere". It is P2 because it is
the most expensive mechanism and should land on a fan-out already proven by US1.

**Independent Test**: Run the same image in two clusters of a three-cluster group, search by image,
and confirm both pods are found with their image strings, the third reports no match, and the cost
was stated before the search ran.

**Acceptance Scenarios**:

1. **Given** an image running in two clusters, **When** the operator searches by image, **Then** both pods are returned with the matching image string and their cluster.
2. **Given** a pod whose **init** container matches, **Then** it is returned, with the match attributed to the init container.
3. **Given** a group of twelve clusters, **When** an image search is submitted, **Then** at most eight clusters are queried concurrently and the UI stated the cost before issuing a request.
4. **Given** a cluster with more matches than the per-cluster bound, **Then** the results are truncated with an explicit marker naming the bound, never silently cut.

---

### User Story 3 - Ask structurally: by label, by kind, by CRD (Priority: P3)

The operator searches by label selector (`app=checkout`), which the API servers evaluate directly,
or asks which clusters in a group even have a kind or CRD (`volumesnapshots.snapshot.storage.k8s.io`),
which discovery answers in one request per cluster. Each term shows which mechanism it will use and
what it costs.

**Why this priority**: These are the cheap, precise queries, and the CRD question is a genuine fleet
answer ("which clusters are missing the CSI snapshot CRDs") that nothing else in the product can
give. P3 because US1's substring sweep covers the common case first.

**Independent Test**: Label an object in one cluster of a group, search the group by that selector,
and confirm one result; then ask for a CRD present in two of three clusters and confirm the answer
names which two.

**Acceptance Scenarios**:

1. **Given** a label selector, **Then** it is sent to each cluster as a selector and the UI states that the API server is doing the filtering.
2. **Given** a CRD presence query, **Then** each cluster is answered from discovery with one request, and the result says which clusters have it and which do not.
3. **Given** a cluster where discovery itself is denied, **Then** it is reported as not permitted — never as "does not have the CRD".
4. **Given** an invalid label selector, **Then** it is rejected before any request is issued, with the parse error verbatim.

### Edge Cases

- A group that matches no cluster (046 FR-005): the search reports that its scope is empty rather than returning an empty result set that looks like "nothing found".
- A member in scope whose context has left the kubeconfig (046 FR-009): reported as unavailable, counted against coverage, and not silently dropped from the denominator.
- Two contexts in scope pointing at the same API server: both searched, both reported; identical objects appear twice, attributed to each context, with no de-duplication (012).
- A searched kind that does not exist in one cluster: that cluster reports the kind as absent, which is distinct from no match and from not permitted.
- A term that is an empty string after trimming: rejected, because an unbounded fleet-wide list is not a search.
- A single cluster that is pathologically slow: it is reported as still in progress and bounded by a per-cluster deadline, and the search as a whole completes with that cluster named as timed out.
- A Secret or ConfigMap whose **name** matches: returned. Its contents are not read, not returned and not matched against.
- 10,000 matching objects in one cluster: bounded, truncated with a marker, and the UI stays responsive to input within the Principle V budget.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: An operator MUST be able to run one search across every member of a fleet group defined by `046`, and MUST be able to narrow the scope to an explicit subset of members.
- **FR-002**: A search MUST run only on explicit submission, MUST NOT be issued per keystroke, and MUST state its scope and the mechanism each term will use before any request is sent.
- **FR-003**: Name and namespace matching MUST be case-insensitive substring matching, and the UI MUST state that this differs from the in-pane filter's prefix matching.
- **FR-004**: A search MUST support, as distinct and labelled mechanisms: label selector (evaluated by the API server), kind or CRD presence (answered from discovery), name/namespace substring (list then filter), and container image including init containers (list pods then inspect).
- **FR-005**: The searchable kinds MUST be a declared bounded set, the operator MUST choose which of them a search covers, and the default MUST be a small subset rather than all of them.
- **FR-006**: Secrets and ConfigMaps MUST be searchable by name, namespace and labels only. No value, key content or secret data may be read, matched, returned, logged, cached or indexed by this feature.
- **FR-007**: Each cluster in scope MUST report exactly one of four outcomes — matched with a count, no match, not permitted with the API server's verbatim reason, or unreachable with the verbatim error — and the absence of a kind MUST be reported distinctly from all four.
- **FR-008**: Every result view MUST state its coverage as members searched out of members in scope, and MUST name every member it did not cover and why. A partial sweep MUST NOT be presented as complete.
- **FR-009**: At most eight clusters MUST be queried concurrently, matching the existing fleet and federation caps.
- **FR-010**: Results MUST be bounded per cluster and in total, and any truncation MUST be marked explicitly with the bound that applied.
- **FR-011**: Each cluster's query MUST be bounded by a deadline; a member that exceeds it MUST be reported as timed out and MUST NOT prevent the search from completing.
- **FR-012**: Results MUST stream in as each cluster answers; the operator MUST NOT wait for the slowest member to see the first result.
- **FR-013**: Cancelling, changing scope, or submitting a new search MUST cancel the in-flight fan-out, stop issuing requests, and discard every result from the superseded search without rendering it.
- **FR-014**: A cancelled search's partial results MUST remain on screen, marked as a cancelled partial with its coverage at the point of cancellation.
- **FR-015**: Selecting a result MUST switch the active context to that result's cluster, MUST state that the switch happened, MUST open the selected object, and MUST leave the result list available.
- **FR-016**: A search MUST introduce no mutating command and MUST NOT offer any action that changes a cluster.
- **FR-017**: An empty or whitespace-only term MUST be rejected, and an invalid label selector MUST be rejected before any request is issued, with the parse error verbatim.
- **FR-018**: A scope that currently contains no member MUST be reported as an empty scope, distinctly from a search that found nothing.
- **FR-019**: In hosted mode the feature MUST work with a scope of one, MUST query as the signed-in user under that user's RBAC, MUST report that user's own coverage, and MUST NOT require relaxing any existing hosted denial.
- **FR-020**: A search MUST start no persistent work: no watches, no retained clients beyond those the fleet already holds, and nothing that continues after the search ends or the view closes.
- **FR-021**: Every new protocol command and event MUST be defined in Rust with generated TypeScript bindings and a serialization round-trip test (Principles II, III).
- **FR-022**: UI logic that decides which results are shown, in what order, and how coverage is summarised MUST have unit tests (Principle III).

### Key Entities

- **Search request**: scope (a fleet group or explicit member list), the kinds covered, and the terms. Carries the mechanism each term resolves to, so the cost is computable before anything is sent.
- **Search term**: one of label selector, kind/CRD presence, name/namespace substring, or image substring. Attributes: the raw text, the resolved mechanism, and its cost class.
- **Per-cluster outcome**: member, status (in progress, matched, no match, not permitted, unreachable, timed out, kind absent), verbatim reason when there is one, result count, and whether its results were truncated.
- **Search result**: member, kind, namespace, name, and the attribution of the match — which term matched and, for an image match, which container and which image string.
- **Coverage**: members searched, members in scope, and the per-member reasons for every gap. Travels with the result list and is never computed by the view from an incomplete list.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: An operator can answer "where is this running across prod" in one submission, without switching context once beforehand.
- **SC-002**: An operator can answer "is this image still running anywhere in the fleet" in one submission, and the answer names every cluster it could not check.
- **SC-003**: Every result view states its own coverage; no screen in the feature can show results without showing how much of the scope they represent.
- **SC-004**: The first result from a reachable cluster appears without waiting for the slowest member of the scope.
- **SC-005**: A search across twelve clusters issues requests to at most eight at a time, and a cancelled search issues no request after the cancellation.
- **SC-006**: Input stays responsive within the Principle V budget while results stream, including with a truncated 10,000-match cluster in scope.
- **SC-007**: No secret or ConfigMap value appears in any result, log line, cache or exported artifact produced by this feature — verified by a test that puts a search term inside a secret's data and asserts it is never matched or returned.
- **SC-008**: A denied cluster is never rendered as "nothing found", verified by a test with a restricted identity against a cluster that does hold a match.

## Assumptions

- `046-fleet-model` has landed; groups and labels exist and a scope can be named.
- The federation tier's counters stay counters. This feature does not add per-cluster object stores, which 034 refused on purpose, and does not read from the federation watches.
- The in-pane filter in `ui/src/resources/search.ts` is unchanged. Its prefix semantics are right for narrowing a visible list; this feature adds a second, differently-labelled surface rather than redefining the first.
- Cross-cluster compare/diff, and any cross-cluster write, are later features that build on this fan-out. Neither is in scope.
- The searchable kind set will grow additively after the feature ships, and each addition carries its own cost statement.
