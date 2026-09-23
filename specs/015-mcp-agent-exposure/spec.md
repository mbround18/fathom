# Feature Specification: MCP Agent Exposure

**Feature Branch**: `015-mcp-agent-exposure`

**Created**: 2026-09-23

**Status**: Draft

**Input**: User description: "Expose Fathom's own command/event protocol surface as a local Model Context Protocol (MCP) server over stdio, so external agent clients (Claude Code, Claude Desktop, or any other MCP-speaking client) running on the same machine can attach and get live, read and gated-mutate cluster context — the same context the in-app Agent panel already uses — from outside the app. Replace the pre-existing, ungated HTTP MCP prototype (`src/mcp_server.rs`) rather than running both. Add an operator-controlled on/off toggle and connection info to Settings, plus a real demo chat launch affordance."

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Attach an external agent client to live cluster context (Priority: P1)

An operator who already uses Claude Code or Claude Desktop for other work wants that same agent to answer questions about the cluster they currently have open in Fathom — "what's crash-looping in namespace X," "show me the last 50 log lines for this pod" — without switching to Fathom's own in-app chat panel. They enable the MCP server in Fathom's Settings, copy the provided connection snippet into their external client's MCP configuration, and the external agent can now call read tools that return the same live, structured cluster data the in-app Agent panel's grounding logic uses.

**Why this priority**: This is the core value proposition of the feature — without it there is nothing to build. Read-only access alone already lets an operator get real value (cluster visibility from any MCP-speaking tool) with the smallest possible blast radius, so it is the right MVP slice.

**Independent Test**: With the MCP server toggled on in Settings and a kube context connected, an external MCP client can list available tools, call a read tool (e.g. "get pod"), and receive live data scoped to what that kube context can see — fully testable without any mutation tooling existing yet.

**Acceptance Scenarios**:

1. **Given** the MCP server toggle is off, **When** the operator opens Settings, **Then** the "Available Agents" section shows the server as not running and no stdio listener process exists.
2. **Given** the operator turns the toggle on, **When** an external MCP client attaches over the provided stdio command, **Then** the client can list tools and successfully call a read tool, receiving data scoped to the resources the currently-connected kube context can access.
3. **Given** the operator turns the toggle off while a client is attached, **When** the client attempts its next tool call, **Then** the call fails cleanly (no crash) and the Settings section reflects the server as stopped.
4. **Given** the currently-connected kube context has no access to a given namespace, **When** an external client calls a read tool scoped to that namespace, **Then** the tool call is denied with a clear, specific reason (not a generic error).

---

### User Story 2 - Perform a gated mutation from an external agent, with real approval (Priority: P2)

An operator using an external agent client wants that agent to be able to actually act — e.g. scale a deployment, delete a stuck pod — instead of only reporting on cluster state. Because mutations are consequential, the operator wants two independent layers of protection: their MCP client's own "allow this tool call?" prompt, and Fathom's own server-side guard (the same one every other mutating command in the app already goes through) that re-checks the target's identity/resource-version before actually touching the cluster.

**Why this priority**: Mutation is valuable but meaningfully riskier than read access, and depends on User Story 1's transport/tooling plumbing already existing. It is P2 because a correct, safe implementation requires the read-path and RBAC-scoping foundations from P1 to already be in place.

**Independent Test**: With the MCP server on, an external client can discover a mutation tool (e.g. "scale deployment"), have its own client-side approval UX gate the call, and — independently of that client-side approval — Fathom's backend guard must reject the mutation if the target's resource-version has changed since the tool was described to the client, proving the server does not trust client-side approval alone.

**Acceptance Scenarios**:

1. **Given** a mutation tool call that has not gone through the client's own approval UX, **When** the MCP server would otherwise execute it, **Then** the tool's declared metadata/schema makes clear to any compliant MCP client that confirmation is required before invocation is destructive — and if a call arrives without the target's current resource-version identity, the backend rejects it rather than mutating.
2. **Given** a stale/mismatched resource-version is presented with a mutation tool call, **When** the backend guard re-validates it, **Then** the mutation is rejected with a clear "target has changed, refresh and retry" style message, and the cluster is not mutated.
3. **Given** a valid, current mutation request scoped within the connected kube context's permissions, **When** the guard validates it, **Then** the mutation executes and an event is emitted the same way any other mutating command's event is emitted.
4. **Given** a mutation tool call targets a namespace/resource the current kube context cannot mutate, **When** the tool is called, **Then** it is denied with a clear reason, regardless of client-side approval.

---

### User Story 3 - See and control the MCP server from Fathom's own Settings (Priority: P1)

An operator wants to know, from inside Fathom itself, whether the MCP server is on, how to connect an external client to it, and whether Fathom already has AI credentials configured (from the existing in-app agent feature) — all in one place — and wants explicit, persistent control over whether the server runs at all, since it's an outbound-facing surface they may not always want active.

**Why this priority**: Without visible, explicit operator control, the feature is unsafe to ship (violates the "explicit, operator-first" principle this app is built on) and undiscoverable. This is foundational alongside User Story 1, not a follow-on.

**Independent Test**: Opening Settings shows the current on/off state and connection snippet without needing any external client attached; toggling it and reopening Fathom (or the panel) shows the persisted state survives, without depending on mutation tooling (User Story 2) at all.

**Acceptance Scenarios**:

1. **Given** the operator opens Settings' "Available Agents" section, **When** the MCP server has never been toggled on, **Then** it shows off by default with a clear explanation and no listener running.
2. **Given** the operator toggles the server on, **When** they restart Fathom, **Then** the toggle's persisted state is on and the server starts accordingly (subject to the same persistence mechanism as other cluster settings).
3. **Given** Anthropic or Ollama credentials are already configured via the existing in-app agent setup, **When** the operator views this section, **Then** it reflects that existing configured state without requiring separate re-entry.

---

### User Story 4 - Launch a real example chat against Fathom's MCP server (Priority: P3)

An operator who is unsure what "attaching an external agent" even looks like wants to see it work with one click, using a real agent CLI already on their machine, rather than reading documentation and configuring it by hand.

**Why this priority**: This is a discoverability/trust-building nicety on top of a fully working P1/P2/P3 core; it depends on the server already running and is valuable but not required for the feature's core value.

**Independent Test**: With a compatible agent CLI installed and the MCP server on, clicking "Launch example chat" starts a real subprocess pre-configured to attach to Fathom's MCP server and opens a visible session; with no compatible CLI installed, the same affordance instead shows an honest "not detected" state with setup instructions — both states independently verifiable without mocking the subprocess.

**Acceptance Scenarios**:

1. **Given** a compatible agent CLI is present on PATH and the MCP server is on, **When** the operator clicks "Launch example chat," **Then** a real subprocess is started, configured to attach to Fathom's MCP server, and its session is visible to the operator.
2. **Given** no compatible agent CLI is detected on PATH, **When** the operator views the affordance, **Then** it clearly states nothing was detected and gives the operator setup instructions rather than a broken or fake button.
3. **Given** the MCP server is off, **When** the operator views the affordance, **Then** it explains the server must be turned on first, rather than attempting to launch a chat against nothing.

---

### Edge Cases

- What happens when the operator disconnects the active kube context entirely while the MCP server is on and a client is attached? All read/mutate tool calls must be denied with a clear "no cluster connected" reason rather than crashing or returning stale data.
- What happens if two external MCP clients attach at once? Both are scoped identically (same kube context, same RBAC resolution); no per-client identity differentiation is required for v1.
- What happens if an external client calls a mutation tool with a target that no longer exists (deleted between tool description and call)? The guard rejects it the same way any other mutating command rejects a vanished target.
- What happens on the very first attach attempt if the operator forgot to enable the toggle? The client's connection attempt fails immediately (no listening process) — the Settings connection snippet must make this precondition obvious.
- What happens to the old HTTP-based MCP prototype's known consumers, if any? None are expected in the shipped app; this spec's replacement removes that surface entirely (see Assumptions).

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: Fathom MUST provide a local MCP server reachable only via stdio (no network/remote listener) that external MCP-speaking clients on the same machine can attach to.
- **FR-002**: The MCP server MUST be off by default and only run while an explicit, operator-controlled, persisted setting is enabled; when disabled, no stdio server process/listener may exist.
- **FR-003**: The MCP server MUST expose read-only tools that mirror the same read surface (resource get/list/describe, logs, events) already assembled for the in-app Agent panel's grounding logic, so external and in-app agents see equivalent cluster context.
- **FR-004**: The MCP server MUST expose gated mutation tools (e.g. scale, delete, apply) whose execution requires the requester's MCP client to have already gated the call through its own approval UX (via clear tool metadata signaling the action is consequential) AND independently requires Fathom's backend to re-validate the target's identity/resource-version through the same guard mechanism every other mutating command in the app uses, before any cluster mutation occurs.
- **FR-005**: Every MCP tool call, read or mutate, MUST be scoped to exactly what the currently-connected kube context is authorized to do, using the app's existing namespace-access resolution; calls outside that scope MUST be denied with a specific, human-readable reason rather than a generic failure.
- **FR-006**: A successful gated mutation performed via an MCP tool call MUST produce the same kind of event/audit trail as an equivalent mutation performed through the existing UI mutation pathway.
- **FR-007**: Fathom's Settings MUST provide an "Available Agents" section showing: whether the MCP server is currently running, the on/off toggle, and a connection snippet/instructions an operator can use to attach an external MCP client.
- **FR-008**: The "Available Agents" section MUST reflect already-known Anthropic/Ollama credential configuration state from the existing in-app agent setup, without requiring separate re-entry or re-detection logic.
- **FR-009**: The "Available Agents" section MUST NOT claim detection of any third-party agent CLI or tool that Fathom cannot actually verify is present on the system.
- **FR-010**: Fathom MUST provide a "Launch example chat" affordance that, when a compatible local agent CLI is genuinely detected on PATH and the MCP server is on, starts a real subprocess pre-configured to attach to Fathom's MCP server and makes that session visible to the operator.
- **FR-011**: When no compatible agent CLI is detected, the "Launch example chat" affordance MUST show an honest not-detected state with setup instructions instead of a non-functional or simulated chat.
- **FR-012**: Fathom MUST remove the pre-existing HTTP-based MCP tool-call prototype (including its ungated mutation tool) entirely, along with all its wiring, rather than operating it alongside the new stdio server.
- **FR-013**: Toggling the MCP server off while a client is attached MUST cause subsequent tool calls from that client to fail cleanly, without crashing Fathom or leaving orphaned listener processes.
- **FR-014**: The system MUST persist the MCP server on/off setting using the same settings-persistence mechanism already used for other cluster-scoped boolean preferences, so the setting survives an app restart.

### Key Entities

- **MCP Server Setting**: A persisted, operator-controlled boolean (plus derived running/stopped status) scoping whether the stdio MCP server process is active; associated with the app's existing cluster/team settings storage.
- **MCP Tool**: A named, described capability exposed to external MCP clients, either read-only or mutating; mutating tools carry metadata marking them as requiring approval and map to an existing backend command/guard.
- **Mutation Guard Context**: The identity/resource-version state captured about a mutation's target at the time it is offered as a tool call, re-validated at execution time before any write occurs.
- **Example Chat Session**: A real, operator-launched external process instance (agent CLI) attached to Fathom's MCP server for demonstration purposes.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: An operator can go from "MCP server off" to "external MCP client successfully reading live cluster data" in under 2 minutes using only the in-app connection instructions.
- **SC-002**: 100% of mutation tool calls that lack a currently-valid target identity/resource-version are rejected before any cluster write occurs, with zero exceptions demonstrated by automated test coverage.
- **SC-003**: 100% of tool calls (read or mutate) targeting resources outside the connected kube context's actual permissions are denied with a specific reason, demonstrated by automated test coverage.
- **SC-004**: When the operator turns the MCP server toggle off, no stdio listener process remains running, verified on every toggle-off in automated coverage.
- **SC-005**: The example-chat affordance never presents a fabricated or non-functional session state — it is either a real, working attach or an honest not-detected message, in 100% of observed states.
- **SC-006**: After this feature ships, zero HTTP endpoints from the old MCP prototype remain reachable in the running app.

## Assumptions

- The pre-existing `src/mcp_server.rs` HTTP-based MCP prototype and its ungated mutation tool have no external production consumers depending on its specific HTTP contract; it predates this feature's security requirements and is fully superseded by the new stdio server, so it can be removed rather than versioned/deprecated alongside the new one.
- "Compatible local agent CLI" for the example-chat affordance means, at minimum, a `claude` binary discoverable on the operator's PATH; other agent CLIs may be considered in future iterations but are not required for v1.
- The existing settings-persistence mechanism used by spec 014's Cluster Settings/Team Profiles panels for boolean preferences is suitable for storing the MCP server's on/off state without modification.
- "The same guard every other mutating command uses" refers to the existing `MutationTarget`-based identity/resource-version validation pattern already used throughout the app's mutation pathway; this feature reuses it rather than defining new mutation semantics.
- RBAC scoping reuses the existing namespace-access resolution already built for the app's RBAC administration feature, rather than introducing a new authorization model specific to MCP.
- Only one external MCP client is expected to be attached at a time in typical usage; the feature does not need to support per-client differentiated permissions in v1.
