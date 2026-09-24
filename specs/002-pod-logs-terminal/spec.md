# Feature Specification: Pod Logs & Terminal

**Feature Branch**: `002-pod-logs-terminal`

**Created**: 2026-09-21

**Status**: Draft

**Input**: User description: "Live pod logs viewer and interactive pod terminal (exec/attach a shell to a running container), wired into the Inspector as new tabs for Pod resources, then showcased in the promo brochure."

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Tail a pod's logs without leaving the app (Priority: P1)

An operator has a pod selected in the Inspector and wants to see what it's printing right now, without shelling out to `kubectl logs -f`.

**Why this priority**: This is the single most common reason anyone opens a Kubernetes GUI mid-incident. The plumbing already exists end to end on the backend; only the viewing surface is missing, so it's the cheapest path to real, immediate value.

**Independent Test**: Select a running pod, open its Logs tab, and see lines appear live as the container produces them — fully testable and valuable on its own, with no dependency on the Terminal story.

**Acceptance Scenarios**:

1. **Given** a pod with a single container is selected in the Inspector, **When** the operator opens the Logs tab, **Then** the most recent log lines appear immediately and new lines continue to stream in as they're produced.
2. **Given** a pod with multiple containers is selected, **When** the operator opens the Logs tab, **Then** they can pick which container's logs to view, and switching containers restarts the stream for the newly chosen one.
3. **Given** logs are streaming and new lines are arriving, **When** the operator scrolls up to read earlier output, **Then** the view stops auto-scrolling to the bottom until they explicitly return to the bottom (e.g. via a "jump to latest" control), so new output doesn't yank their scroll position.
4. **Given** the Logs tab is open and streaming, **When** the operator switches to a different tab or closes the Inspector, **Then** the log stream stops on the backend (no orphaned stream left running).
5. **Given** a pod has crashed or its container has stopped, **When** the operator opens the Logs tab, **Then** they see the last available output plus a clear "stream ended" indicator rather than a silent blank pane.

---

### User Story 2 - Open an interactive shell inside a running container (Priority: P2)

An operator needs to poke around inside a container — check a config file, run a diagnostic command, inspect a mounted volume — without leaving the app or remembering pod/namespace names for a separate `kubectl exec`.

**Why this priority**: High value but meaningfully riskier and more complex than read-only log tailing (it executes commands inside a live workload), and depends on nothing from Story 1 — it can ship after logs are solid.

**Independent Test**: Select a running pod, open its Terminal tab, type a command (e.g. `ls`), and see the real output from inside the container — independently testable and valuable without the Logs tab existing.

**Acceptance Scenarios**:

1. **Given** a running pod with a single container is selected, **When** the operator opens the Terminal tab, **Then** an interactive shell session attaches to that container and the operator can type commands and see their output.
2. **Given** a pod with multiple containers is selected, **When** the operator opens the Terminal tab, **Then** they choose which container to attach to before the session starts.
3. **Given** an active terminal session, **When** the operator resizes the pane or window, **Then** the remote shell's terminal size updates to match, so full-screen tools (e.g. `top`, `vi`) render correctly.
4. **Given** an active terminal session, **When** the operator closes the tab, switches away, or closes the Inspector, **Then** the shell session is terminated on the backend (no orphaned exec session left running).
5. **Given** the operator opens the Terminal tab against a pod whose container has no usable shell, **When** the attach attempt fails, **Then** the operator sees a clear error explaining the attach failed, not a silently empty pane.
6. **Given** an operator without exec permission in the cluster (RBAC denies `pods/exec`), **When** they attempt to open the Terminal tab, **Then** they see a clear permission-denied message rather than a hang or generic error.

> **Note (024-pod-exec-terminal)**: scenario 1 no longer attaches immediately. A `pods/exec` permission pre-check and an explicit confirmation now precede every session; see `specs/024-pod-exec-terminal/spec.md`.

---

### User Story 3 - See both capabilities in the promo brochure (Priority: P3)

A prospective user or evaluator reading Fathom's promotional brochure should see that live log tailing and an in-app terminal are real, working features — not just take the claim on faith.

**Why this priority**: Marketing value only; depends entirely on Stories 1 and 2 already existing and working. Lowest priority because it produces no end-user functionality on its own.

**Independent Test**: Regenerate the promo brochure and confirm it includes a real screenshot of the Logs tab actively tailing output from the live demo cluster, plus a page showcasing the Terminal capability, without altering or endangering the demo cluster's state.

**Acceptance Scenarios**:

1. **Given** the promo capture pipeline runs against the live demo cluster, **When** it reaches the logs scene, **Then** it opens a pod's Logs tab, waits for real streamed lines to appear, and captures a screenshot — using only the existing read-only, non-destructive capture mechanism.
2. **Given** the promo capture pipeline is read-only and must never execute commands inside a live cluster pod, **When** the brochure is built, **Then** the Terminal capability is showcased without ever attaching a real interactive shell during an unattended capture run against the live cluster.
3. **Given** the brochure is regenerated, **When** an evaluator reads it, **Then** the new pages follow the brochure's existing visual language (branded header/footer/watermark, page numbering) and read as consistent with the rest of the document.

### Edge Cases

- What happens when a pod is deleted while its Logs or Terminal tab is open? The stream/session must end and the tab must show a clear "no longer available" state, not hang or error-loop.
- What happens when the operator opens Logs or Terminal, then immediately navigates away before the stream/session finishes attaching? The in-flight attach must be cancelled/stopped rather than continuing to run unattended.
- What happens when the network to the cluster drops mid-session for either feature? The tab must surface a disconnected state distinct from "still loading."
- What happens if the operator tries to open a second Terminal session while one is already active (e.g. against a different pod)? The prior session must be cleanly ended before the new one starts, per the one-active-session assumption below.
- What happens when a container produces extremely high-volume log output? The view must remain responsive and bounded (not attempt to hold unlimited history in memory).

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: The Inspector MUST offer a "Logs" tab when the inspected resource is a Pod, and MUST NOT offer it for any other resource type.
- **FR-002**: Opening the Logs tab MUST begin streaming that pod's log output within a few seconds under normal conditions, showing recent history plus new lines as they arrive.
- **FR-003**: When a pod has more than one container, the Logs tab MUST let the operator choose which container's logs to stream, defaulting to a reasonable first choice (e.g. the pod's first container).
- **FR-004**: The Logs tab MUST auto-scroll to show the newest output by default, MUST stop auto-scrolling when the operator scrolls up to read history, and MUST offer an explicit way to resume following the newest output.
- **FR-005**: Leaving the Logs tab (switching tabs, closing the Inspector, or selecting a different resource) MUST stop the underlying log stream so no stream keeps running unattended.
- **FR-006**: If the log stream ends (container stopped, pod deleted, connection lost), the Logs tab MUST show a clear, distinct indicator for that state rather than leaving the view looking merely idle.
- **FR-007**: The Inspector MUST offer a "Terminal" tab when the inspected resource is a Pod and the pod is in a running state, and MUST NOT offer it for pods that are not running (e.g. pending, terminated, completed).
- **FR-008**: When a pod has more than one container, the Terminal tab MUST let the operator choose which container to attach to before starting the session.
- **FR-009**: Opening the Terminal tab MUST attach an interactive shell to the chosen container and let the operator send input and see the shell's output in real time.
- **FR-010**: The Terminal MUST track the visible terminal pane's size and propagate size changes to the remote shell session so full-screen terminal programs render correctly.
- **FR-011**: Leaving the Terminal tab (switching tabs, closing the Inspector, or selecting a different resource) MUST terminate the underlying shell session so no session keeps running unattended.
- **FR-012**: Only one Terminal session may be active at a time across the whole application; starting a new one MUST cleanly end any previous one first.
- **FR-013**: If attaching a Terminal session fails (no usable shell in the container, insufficient permissions, connection error), the operator MUST see a clear, specific-enough error rather than a blank or silently non-functional pane.
- **FR-014**: The promo brochure's capture pipeline MUST be able to demonstrate the Logs tab against the live demo cluster using only its existing read-only capture mechanism, with no change to that mechanism's guarantee that no cluster-affecting command is ever sent during capture.
- **FR-015**: The promo brochure's capture pipeline MUST showcase the Terminal capability without ever attaching a real interactive shell session to the live demo cluster during an unattended capture run.
- **FR-016**: The regenerated promo brochure MUST present the new Logs and Terminal pages/shots using the brochure's existing branded page treatment (header, footer, watermark, page numbering) consistent with its other pages.

### Key Entities

- **Log Stream**: A live, ordered sequence of text lines from one pod's chosen container, bounded in how much history it retains client-side, with a lifecycle (streaming / ended / error) tied to the Logs tab being open.
- **Terminal Session**: A single active interactive shell attached to one pod's chosen container, carrying keystroke input to the container and shell output back, aware of the visible terminal's size, with a lifecycle (attaching / active / ended / error) tied to the Terminal tab being open, and limited to at most one active instance at a time application-wide.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: An operator can go from selecting a running pod to seeing its live log output on screen in under 5 seconds under normal cluster conditions.
- **SC-002**: An operator can go from selecting a running pod to typing a command in an attached terminal and seeing its output in under 10 seconds under normal cluster conditions.
- **SC-003**: Closing or navigating away from the Logs tab or Terminal tab leaves zero streaming sessions running in the background, 100% of the time.
- **SC-004**: The regenerated promo brochure includes at least one genuine, non-mocked screenshot demonstrating live log output and at least one page showcasing the terminal capability, with zero cluster-affecting commands issued during an unattended capture run.

## Assumptions

- Only one Terminal session is supported application-wide at a time; opening a second one ends the first. Multiple concurrent terminals is out of scope for this feature.
- The Logs tab reuses the log-streaming capability that already exists end-to-end on the backend; this feature is about giving it a UI, not redesigning how logs are fetched.
- "Running" for Terminal-tab eligibility means the pod (and, if applicable, the chosen container) is in a state where a shell process could plausibly be attached — pods that are pending, completed, or terminated are excluded.
- The promo brochure's read-only capture guarantee (no cluster-affecting commands during an unattended run) is treated as a hard constraint carried over from the existing brochure pipeline, not something this feature may relax.
- Client-side log history is bounded (not unlimited) to keep the view responsive under high-volume output; exact retention size is an implementation detail, not a user-facing requirement.
- This feature does not change any existing write-action confirmation/safety model (e.g. delete, scale, restart) — a terminal session is a new, separate capability with its own lifecycle rules defined above.
