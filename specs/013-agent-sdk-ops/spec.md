# Feature Specification: Agent SDK Ops (AI-Assist Troubleshooting)

**Feature Branch**: `013-agent-sdk-ops`

**Created**: 2026-09-22

**Status**: Draft

**Input**: User description: "Tier E — Agent SDK ops (AI-assist troubleshooting): an in-app 'what's wrong with this pod/cluster' assistant built on the Claude Agent SDK, grounded in data already streamed to the store (events, conditions, logs, pod-lifecycle detail), with per-operator auth (BYOK Anthropic key or local Ollama), built against the Agent SDK's public interface so a future hosted mode is a config path not a rewrite. Explicit non-goal: autonomous/unattended mutation — every agent-proposed action goes through the same diff-preview + confirmation gate as a human-initiated one."

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Ask the assistant what's wrong with a pod (Priority: P1)

An operator is looking at a pod that's crash-looping or stuck pending. Instead of manually cross-referencing events, conditions, container statuses, and logs, they open an assistant panel scoped to that pod, ask "what's wrong with this pod?", and receive a plain-language diagnosis grounded in the exact data already visible in the pod inspector (events, conditions, restart counts, log tail, lifecycle detail).

**Why this priority**: This is the entire value proposition of Tier E and the smallest possible slice that is independently useful — read-only diagnosis with zero mutation risk. Everything else (suggested actions, execution) builds on this.

**Independent Test**: Can be fully tested by selecting a pod with a known failure mode (e.g. `ImagePullBackOff`, `CrashLoopBackOff`, failing readiness probe), opening the assistant panel, asking a free-text question, and verifying the response cites only data present in the pod's existing events/conditions/logs/status — never fabricated fields — and issues zero backend mutating commands.

**Acceptance Scenarios**:

1. **Given** a pod in `CrashLoopBackOff` with events and a log tail already visible in its inspector, **When** the operator opens the assistant panel for that pod and asks what's wrong, **Then** the assistant's response references specific events, conditions, or log lines drawn from that pod's already-streamed data, and the panel shows which data sources (events, conditions, logs, container statuses) were read to ground the answer.
2. **Given** the assistant is generating a response, **When** the operator views the panel, **Then** no cluster-mutating backend command is issued at any point during question-asking or answer-generation — the conversation is provably read-only.
3. **Given** a pod with no anomalies (all conditions `True`, no warning events), **When** the operator asks what's wrong, **Then** the assistant states the pod appears healthy rather than fabricating a problem, and does not propose any action.
4. **Given** the assistant cannot reach its configured model (auth not configured, network unreachable, local model not running), **When** the operator asks a question, **Then** the panel shows a specific, verbatim-forwarded error (per Principle I) explaining why and how to fix it (e.g. link to auth setup), never a silent failure or a generic "something went wrong."

---

### User Story 2 - Operator configures auth mode (BYOK key or local Ollama) (Priority: P1)

Before using the assistant for the first time, an operator chooses how it runs: enter their own Anthropic API key (stored locally, used only for their own requests), or point it at a local Ollama instance for fully offline operation with no external network calls. They can change this choice later, and can also fully disable the assistant.

**Why this priority**: The assistant cannot answer anything without an auth mode configured, and the "no forced cloud dependency" promise is a first-class product requirement (Tier A), not an afterthought — it must ship in the same slice as the first working conversation, not after.

**Independent Test**: Can be fully tested by opening assistant settings with no auth configured (assistant panel shows a setup prompt, no request possible), entering a BYOK Anthropic key and confirming a request succeeds, then switching to local Ollama mode and confirming requests are answered via the local endpoint with zero calls to any external API.

**Acceptance Scenarios**:

1. **Given** no auth mode is configured, **When** the operator opens the assistant panel, **Then** they see a setup prompt offering "Enter Anthropic API key" or "Use local Ollama" and cannot submit a question until one is configured.
2. **Given** the operator enters an Anthropic API key, **When** they save it, **Then** the key is stored the same way other credentials in `~/.fathom/` are stored (never logged, never echoed back in full through the protocol, masked in the UI unless explicitly revealed) and a subsequent question successfully reaches the configured model.
3. **Given** the operator selects local Ollama mode, **When** they save it, **Then** all assistant requests are sent only to the configured local Ollama endpoint, and the app makes no calls to any hosted LLM API for the duration that mode is active.
4. **Given** local Ollama mode is selected but the local endpoint is unreachable or the configured model is not pulled, **When** the operator asks a question, **Then** the panel reports a specific error (endpoint unreachable / model not found) with a concrete remediation hint, not a generic failure.
5. **Given** an auth mode is configured, **When** the operator disables the assistant entirely, **Then** no assistant-related requests are made and the panel returns to a disabled/opt-in state; existing conversation history is retained locally unless the operator explicitly clears it.

---

### User Story 3 - Assistant proposes an action, operator reviews and confirms it through the standard gate (Priority: P2)

While diagnosing a problem, the assistant identifies a specific corrective action (e.g. "restart this pod," "scale this deployment to 3 replicas," "roll back this Helm release to revision 4"). It presents this as a distinct, clearly-labeled suggestion — never performs it. The operator reviews the suggestion, and if they want to proceed, it opens the exact same diff-preview and confirmation dialog used when the operator initiates that action manually, naming the same affected objects. Only after the operator explicitly confirms does the action execute, through the same backend command path as a manually-typed action.

**Why this priority**: This is where Principle I ("Operator-First, Explicit Mutation") does the real work in this feature, and it's the part most likely to be gotten wrong (auto-execution, watered-down confirmation, a parallel mutation path). It depends on User Story 1 (diagnosis) being in place first, so it's sequenced second, but it's still core to Tier E's stated non-goal of autonomous mutation.

**Independent Test**: Can be fully tested by prompting the assistant into suggesting a known mutating action (e.g. "restart pod X"), verifying the suggestion renders as inert text/structured content with no execution occurring, clicking "review" to open the standard confirmation dialog for that exact action type, verifying the diff-preview/object list matches what a manually-initiated version of the same action would show, confirming it, and observing the mutation applied through the existing command path — then separately verifying that declining or closing the dialog leaves the cluster untouched.

**Acceptance Scenarios**:

1. **Given** the assistant's response includes a suggested action, **When** the operator reads the response, **Then** the suggestion is visually and structurally distinct from informational text (e.g. an actionable card naming the exact object(s) and operation), and no backend mutating command has been issued merely by the suggestion appearing.
2. **Given** a suggested action is shown, **When** the operator selects "review," **Then** the app opens the same confirmation UI component used for that action when initiated manually (e.g. the existing delete/scale/restart/Helm-mutate confirmation dialogs), populated with a diff-preview or affected-object list computed fresh against current cluster state at review time — never a stale snapshot from when the assistant generated the suggestion.
3. **Given** the confirmation dialog is open from an assistant suggestion, **When** the operator confirms, **Then** the action is executed through the identical backend command path (and `MutationTarget` guard classification) used for a human-initiated instance of that command, and the conversation reflects that the action was taken and its result (success or the verbatim backend error).
4. **Given** the confirmation dialog is open from an assistant suggestion, **When** the operator cancels or closes it, **Then** no cluster state changes, and the assistant treats the action as declined (it may be asked again, but does not repeat or auto-retry it).
5. **Given** the assistant proposes an action outside what the operator's current context/permissions can do (e.g. a cluster-scoped action while namespace-restricted, per RBAC context), **When** the operator opens the review, **Then** the same failure the manual path would show is surfaced — the assistant path never has broader effective permissions than the manual path.
6. **Given** any assistant output whatsoever, **When** examined for any code path in the system, **Then** there exists no mechanism by which an assistant-proposed action reaches the backend's mutating command path without first passing through this identical confirmation step — confirmed by the absence of any "auto-apply," "auto-confirm," or "high-confidence bypass" code path.

---

### User Story 4 - Operator reviews and continues a prior assistant conversation (Priority: P3)

An operator returns to a pod or workload they were troubleshooting earlier and wants to pick the conversation back up rather than re-asking from scratch, including the eventual outcomes of any actions they confirmed during that session.

**Why this priority**: Useful for continuity across a troubleshooting session but not required for the assistant to deliver its core value — informational diagnosis and gated action review work fine as a single-session experience without persistence. Sequenced last.

**Independent Test**: Can be fully tested by having a conversation about a pod (including one confirmed action), closing the panel or restarting the app, reopening the assistant on the same pod, and confirming the prior conversation (including the recorded outcome of the confirmed action) is still visible.

**Acceptance Scenarios**:

1. **Given** an operator had a prior conversation scoped to a specific object, **When** they reopen the assistant panel on that same object, **Then** the prior conversation (messages, suggested actions, and recorded confirm/decline/execution outcomes) is shown.
2. **Given** stored conversation history, **When** the operator explicitly clears it, **Then** it is permanently removed and does not reappear on restart.
3. **Given** the app restarts, **When** the operator reopens a conversation, **Then** no assistant request is automatically replayed or re-sent — history is inert until the operator asks something new.

---

### Edge Cases

- What happens if the operator asks a question unrelated to cluster troubleshooting (e.g. general chit-chat)? The assistant MUST stay scoped to its grounding data and decline or redirect rather than answering as a general-purpose chatbot (explicit non-goal).
- What happens if the grounding data for the object in question is incomplete or stale (e.g. log stream not started, events truncated by the app's bounded ring buffer per Principle V)? The assistant MUST state what data it does and does not have rather than presenting an answer as if it had complete information.
- What happens if the assistant proposes an action, and before the operator confirms, the target object's state changes (e.g. the pod is deleted by another actor, or already restarted)? The confirmation step MUST recompute against current state at confirm-time and fail clearly (matching the 006-helm-mutate precedent) rather than applying a stale proposal.
- What happens if the operator has a BYOK key configured but it becomes invalid (revoked, rate-limited, expired) mid-session? The panel MUST surface the verbatim backend/API error and prompt re-entry of a valid key, not retry silently or fall back to a different mode without the operator's consent.
- What happens if local Ollama mode is active and the selected model cannot fit the grounding context (e.g. very long log tail)? The system MUST truncate/summarize what it sends with a clear indicator that truncation occurred, rather than silently dropping data or failing without explanation.
- What happens if the operator switches auth mode mid-conversation? The existing conversation history remains visible; only subsequent messages use the newly selected mode, and the panel indicates the mode switch occurred.
- What happens if two assistant conversations are open concurrently for different objects? Each is independently scoped to its own object's grounding data; one conversation's suggested actions never apply to another conversation's object.
- What happens when the operator's RBAC/context lacks permission for a suggested action? Same as any manual attempt at that action — the confirmation flow surfaces the backend's permission error verbatim; the assistant does not attempt to work around it.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: System MUST provide an in-app assistant surface, scoped per-object (starting with pods, extensible to other workload types), that answers "what's wrong with this [object]?"-style questions using only data already available through the app's existing protocol events/store (events, conditions, logs, container/pod status and lifecycle fields) for that object — no separate scraping, polling, or ingestion path bypassing the existing store.
- **FR-002**: System MUST perform all agent orchestration and model/API calls (BYOK or local) from the backend; the UI MUST communicate with the assistant only through the app's versioned command/event protocol, never by calling an LLM provider or the Agent SDK directly from UI code (Principle II).
- **FR-003**: System MUST let the operator choose, per-install, between at least two auth modes: (a) a self-provided Anthropic API key entered and stored locally, or (b) a local Ollama endpoint/model for fully offline operation with zero external network calls while that mode is active. The assistant MUST be unusable (blocked, with a setup prompt) until one mode is configured.
- **FR-004**: System MUST store any BYOK API key using the same credential-handling guarantees as other secrets in `~/.fathom/` (Principle VI): never logged, never included in protocol payloads that reach a display surface in plaintext, masked in the UI by default, and only shown when the operator explicitly reveals it.
- **FR-005**: System MUST integrate with the model backend exclusively through the Claude Agent SDK's public interface (not internal/undocumented APIs), such that adding a future hosted/cloud-ops auth mode requires only a new auth/config path and not a rewrite of the orchestration logic.
- **FR-006**: System MUST distinguish, in both the protocol and the UI, between three distinct states: (1) informational assistant text with no proposed action, (2) a suggested action (a structured proposal naming a specific operation and target object(s)) that has not been reviewed or confirmed, and (3) an executed action that has passed through confirmation and been sent to the backend's mutating command path. A suggested action MUST NOT be serialized or treated identically to an executed one at any layer.
- **FR-007**: System MUST NOT execute any cluster-mutating command as a direct consequence of assistant output. Every suggested action MUST require the operator to explicitly open a review/confirmation step before anything is sent to the mutating command path — there MUST be no auto-apply, auto-confirm, or confidence-based bypass of this step, regardless of how the assistant frames its own certainty.
- **FR-008**: When an operator chooses to review a suggested action, System MUST reuse the exact same confirmation UI component and diff-preview/affected-object-list mechanism already used for that action type when initiated manually (e.g. delete, scale, restart, Helm upgrade/rollback/uninstall confirmations) — not a separate or abbreviated assistant-specific confirmation flow.
- **FR-009**: System MUST recompute the diff-preview or affected-object state fresh, against the cluster's current state, at the moment the operator opens the confirmation review — never reusing a stale snapshot captured when the assistant first generated the suggestion.
- **FR-010**: Upon confirmation, System MUST route the resulting mutation through the identical backend command path (including the existing `MutationTarget` guard classification) used for a human-initiated instance of that same command — no parallel or agent-specific mutation path may exist.
- **FR-011**: System MUST report the outcome (success, or the verbatim backend error) of a confirmed assistant-proposed action back into the conversation, using the app's standard error-forwarding behavior (Principle I: backend errors reach the operator verbatim, never replaced with generic messages).
- **FR-012**: System MUST make clear to the operator, for every assistant response, which underlying data sources (events, conditions, logs, lifecycle detail, etc.) were used to ground that specific answer.
- **FR-013**: System MUST allow the operator to fully disable the assistant feature; when disabled, no assistant-related requests (to a hosted API or a local endpoint) are made by the application.
- **FR-014**: System MUST decline or redirect questions unrelated to cluster/object troubleshooting rather than answering as a general-purpose assistant, keeping the feature scoped to its stated purpose.
- **FR-015**: System MUST surface a specific, actionable error (not a generic failure) when the configured auth mode cannot be reached or used — covering at minimum: missing/invalid BYOK key, unreachable local Ollama endpoint, and local model not available.
- **FR-016**: System MUST NOT grant assistant-initiated actions any permission, RBAC scope, or capability beyond what the operator's own current cluster context already allows; a permission failure on a confirmed action MUST surface the same way it would for a manually-initiated equivalent.
- **FR-017**: System MUST persist assistant conversation history (messages, suggested actions, and their confirm/decline/execution outcomes) locally per-object, allow the operator to view it on return, and allow the operator to explicitly clear it; no assistant request is automatically replayed on app restart.
- **FR-018**: System MUST bound and clearly indicate any truncation or summarization applied to grounding data (e.g. long log tails) before it is sent to the model, so the operator can tell when an answer may be based on partial data.

### Key Entities

- **Assistant Conversation**: A per-object (e.g. per-pod) sequence of operator questions and assistant responses, scoped to that object's grounding data; persisted locally; includes zero or more suggested actions and their eventual outcomes.
- **Assistant Message**: A single turn in a conversation — either operator-authored (a question) or assistant-authored (informational text and/or a suggested action), tagged with which grounding data sources informed it.
- **Suggested Action**: A structured proposal (operation type + target object(s), e.g. "restart pod X" or "roll back Helm release Y to revision N") generated by the assistant; inert until reviewed; distinct from, and never automatically promoted to, an Executed Action.
- **Executed Action**: The record of a Suggested Action that the operator reviewed via the standard confirmation flow and confirmed; references the backend command that was actually issued and its result.
- **Auth Configuration**: The operator's chosen assistant auth mode (BYOK Anthropic key, or local Ollama endpoint/model) and its associated locally-stored credential/config, following the same handling as other stored credentials.
- **Grounding Data Reference**: A pointer to the specific already-streamed protocol data (events, conditions, logs, lifecycle fields) that a given assistant answer drew from, shown to the operator for traceability.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: An operator can get a grounded diagnosis of a failing pod (citing specific events/conditions/logs from that pod) in under 30 seconds from opening the assistant panel, without leaving the pod's context.
- **SC-002**: 100% of assistant conversations issue zero cluster-mutating backend commands purely from question-asking or response-generation — mutation only ever occurs after the standard confirmation step (verifiable by code-path audit and E2E test, per Principle III).
- **SC-003**: 100% of assistant-suggested actions that reach execution pass through the exact same confirmation UI component and diff-preview mechanism as their manually-initiated equivalent — zero assistant-specific or abbreviated confirmation paths exist.
- **SC-004**: 100% of BYOK credentials entered are never observable in application logs, protocol traces, or exported diagnostics in plaintext.
- **SC-005**: An operator can complete first-time assistant setup (choose auth mode, enter a key or point to local Ollama, and get a successful first answer) in under 2 minutes.
- **SC-006**: 0% of assistant responses to off-topic (non-troubleshooting) questions are answered as general conversation — all are declined or redirected.
- **SC-007**: An operator using local Ollama mode can confirm (e.g. via network inspection) that 0 requests leave the machine to any hosted LLM API during that mode's use.

## Assumptions

- The assistant's v1 scope is per-object troubleshooting (starting with pods; extensible to nodes, deployments, and Helm releases using the same pattern) rather than a whole-cluster free-roaming agent — this matches the "what's wrong with this pod/cluster" framing in docs/MVP.md §14 Tier E while keeping the grounding data model tractable for v1.
- The richer pod-lifecycle/probe/hook/init-container detail being spec'd in parallel (008-pod-lifecycle-diagnostics, docs/MVP.md §7) is treated as an additional grounding data source that becomes available once shipped; this feature does not depend on its specific field shapes today and must degrade gracefully to whatever lifecycle data currently exists in the protocol.
- The existing debug-facing MCP server (`src/mcp_server.rs`, exposing Fathom's own running state to an external assistant like Claude Desktop) is a separate, already-shipped feature (Tier A) with a different consumer (external tooling introspecting Fathom itself). This feature is net-new: an in-app agent, using the Agent SDK, that helps the operator troubleshoot their *cluster*. It may reuse the same "expose grounding data via MCP-style tools" pattern internally, but is not the same surface and does not modify `src/mcp_server.rs`.
- "Local Ollama" is assumed to mean an Ollama instance already running and reachable by the operator (locally or on their network); this feature is responsible for connecting to and using it, not for installing, bundling, or managing the Ollama runtime itself.
- A future hosted/cloud-ops auth mode (mentioned in docs/MVP.md as a later possibility) is explicitly out of scope for this spec; the only requirement carried forward is that today's Agent-SDK-public-interface integration must not preclude adding it later without a rewrite.
- Conversation history is stored locally per-object (e.g. keyed by namespace/kind/name) in `~/.fathom/`, following the same on-disk persistence and upgrade-safety guarantees (Principle VI) as other app state; it is not synced to any external service.
- v1 ships with the assistant capable of proposing actions only within the set of mutating commands the app already exposes through its existing protocol/confirmation UI (delete, scale, restart, port-forward changes, YAML apply, Helm upgrade/rollback/uninstall, etc.) — it does not introduce new kinds of mutations that don't already have a manual confirmation flow to reuse.
- Multi-step or chained remediation (the assistant executing several actions in sequence toward a goal) is out of scope for v1; each suggested action is reviewed and confirmed individually, one at a time — this follows directly from the stated non-goal of autonomous/unattended mutation.
- Rate limiting / cost control for BYOK usage is the operator's own responsibility via their Anthropic account in v1 (matching "self-hosted-first, no forced cloud dependency"); the app surfaces provider-reported errors (e.g. rate limit hit) verbatim rather than implementing its own quota system.
