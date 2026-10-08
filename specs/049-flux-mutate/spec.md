# Feature Specification: Flux mutate (suspend, resume, reconcile)

**Feature Branch**: `049-flux-mutate`

**Created**: 2026-10-08

**Status**: Draft

**Input**: User description: "on the flux can we get a more rich integration like suspending or resuming?"

## Clarifications

### Session 2026-10-08

- Q: How far does the mutation go? → A: Suspend, resume and reconcile — the three verbs operators actually use from the `flux` CLI — on sources (GitRepository, OCIRepository, HelmRepository, Bucket), Kustomizations and HelmReleases. Reconcile offers a "with source" option that reconciles the referenced source first. Chosen over a read-only view (today's state) and over full parity with the `flux` CLI (create, delete, export, diff), which are out of scope.
- Q: Does the confirm step preview anything? → A: Yes. The confirm dialog names the object by kind, namespace and name, says what the verb does to it, and for a Kustomization or HelmRelease being suspended lists what it manages (from Flux's own inventory / release) so the operator sees what stops being reconciled. For a source being suspended it lists the Kustomizations and HelmReleases that consume it.
- Q: Is this allowed in hosted mode? → A: Yes. Each verb patches one namespaced object the signed-in user's own RBAC already governs, exactly like a CronJob suspend or a Helm rollback. Every apply is audited.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Suspend and resume a Flux resource (Priority: P1)

An operator is about to hand-patch a Deployment during an incident. They open the Flux pane, pick the Kustomization that owns that Deployment, and choose Suspend. A dialog names the Kustomization, explains that Flux will stop applying its source until resumed, and lists the objects it manages. They confirm. The row and the inspector show the resource as suspended within a few seconds, and the audit log records who suspended what. After the incident they choose Resume on the same resource, confirm, and watch it reconcile back to Ready.

**Why this priority**: Without suspend, every hand-fix on a GitOps-managed cluster is reverted at the next interval. This is the single most common reason an operator reaches for the `flux` CLI, and it is the reason the user asked.

**Independent Test**: On the fake backend, open the Flux pane, suspend a Kustomization, assert the confirm dialog names it and lists its inventory, confirm, assert exactly one mutation command was sent with `suspend: true`, inject the applied event and assert the row reads Suspended. Resume reverses it with `suspend: false`.

**Acceptance Scenarios**:

1. **Given** a Ready Kustomization, **When** the operator chooses Suspend and confirms, **Then** one command naming that kind, namespace and name with suspend set is sent, and nothing is sent before confirmation.
2. **Given** a Suspended HelmRelease, **When** the operator chooses Resume and confirms, **Then** one command with suspend cleared is sent and the inspector reflects the resumed state after the backend replies.
3. **Given** the confirm dialog for a Kustomization with an inventory, **Then** the dialog lists the managed objects by kind and name.
4. **Given** the confirm dialog for a source, **Then** the dialog lists every Kustomization and HelmRelease that references it.
5. **Given** the operator cancels the dialog, **Then** no command is sent.

---

### User Story 2 - Reconcile now, optionally with its source (Priority: P2)

An operator has just pushed a commit and does not want to wait out a ten-minute interval. They pick the Kustomization, choose Reconcile, tick "with source" so the GitRepository fetches first, confirm, and watch the revision advance and the Ready condition flip through Reconciling back to Ready. The same works on a HelmRelease (with its HelmRepository or OCIRepository) and on a source by itself.

**Why this priority**: The second most common `flux` CLI verb. It depends on nothing in US1, but is less urgent than suspend because waiting is always an option.

**Independent Test**: On the fake backend, reconcile a HelmRelease with source, assert one command carrying `with_source: true`, inject the applied event and a details update with a new revision, and assert the inspector shows the new revision and a success toast.

**Acceptance Scenarios**:

1. **Given** any Flux resource, **When** the operator chooses Reconcile and confirms, **Then** one reconcile command naming it is sent.
2. **Given** a Kustomization or HelmRelease, **When** "with source" is ticked, **Then** the command carries that flag and the backend requests the source's reconciliation before the resource's own.
3. **Given** a source, **Then** the "with source" option is not offered.
4. **Given** a suspended resource, **When** the operator chooses Reconcile, **Then** the dialog warns that Flux ignores reconcile requests while suspended and offers Resume instead.

---

### User Story 3 - Reach any Flux resource from the pane (Priority: P3)

An operator opens the Flux pane and sees every source, Kustomization and HelmRelease, not only the ones that are not Ready. Clicking a row opens that resource in the inspector where the verbs live. Rows are grouped by kind and carry their ready tone and revision.

**Why this priority**: Today the pane lists only unhealthy resources and the inspector is reachable only through a Helm release's origin link. A Ready Kustomization that the operator wants to suspend is unreachable without this.

**Independent Test**: On the fake backend with a Flux overview containing Ready and failed resources, open the pane, assert every resource is listed, click a Ready Kustomization and assert the inspector opens with the Suspend and Reconcile verbs visible.

**Acceptance Scenarios**:

1. **Given** an installed Flux overview, **When** the pane opens, **Then** every source, Kustomization and HelmRelease is listed, grouped, with not-Ready rows first.
2. **Given** a listed row, **When** it is clicked, **Then** the inspector opens on that resource with its verbs.

---

### Edge Cases

- The object was deleted between the pane listing and the confirm: the backend reports the failure by name and the UI shows an error toast; nothing else changes.
- The object is already in the requested state (suspending a suspended resource): the backend still applies the patch (idempotent) and reports success; the UI disables the verb that would be a no-op so this is reachable only through a stale view.
- Reconcile with source when the source is in another namespace (`spec.sourceRef.namespace`): the backend resolves the source's namespace from the reference, falling back to the resource's own namespace.
- Reconcile with source when the source kind is not served (CRD removed): the backend reports which source could not be found and does not request the resource's own reconcile.
- RBAC forbids the patch: the backend relays the API server's message verbatim; the audit log records the attempt as failed.
- Hosted mode: every verb is allowed and audited; the signed-in user's RBAC decides.
- Flux not installed: no verbs are reachable because the pane shows the existing "Not installed" state.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: The protocol MUST offer a single command that suspends or resumes one Flux resource identified by kind, namespace and name, and a single command that requests reconciliation of one Flux resource with an optional with-source flag.
- **FR-002**: Suspend and resume MUST be implemented as a merge patch of `spec.suspend` on the discovered served version of the CRD; reconcile MUST set the `reconcile.fluxcd.io/requestedAt` annotation to the current RFC 3339 time, on the source first when with-source is set.
- **FR-003**: After a successful apply the backend MUST emit an applied event naming the resource and the verb, re-read the resource and emit its details, and refresh the Flux overview so the pane and inspector converge without waiting for the poll.
- **FR-004**: Failures MUST be reported through a failed event carrying the resource identity, the verb and the API server's message.
- **FR-005**: Both commands MUST be classed as namespaced mutations, carry an audit summary of kind `FluxControl`, and be allowed in hosted mode.
- **FR-006**: Every new command and event variant MUST have a serde round-trip test.
- **FR-007**: The UI MUST require explicit confirmation naming the object before sending either command, and MUST send nothing when the dialog is cancelled.
- **FR-008**: The confirm dialog for suspending a Kustomization or HelmRelease MUST list what the resource manages; for a source it MUST list the Kustomizations and HelmReleases that reference it. The summary therefore carries the managed-object list from Flux's inventory.
- **FR-009**: The Flux pane MUST list every resource with a click-through to the inspector, grouped by kind, not-Ready first.
- **FR-010**: The inspector MUST offer Suspend or Resume (whichever applies) and Reconcile; Kustomizations and HelmReleases MUST additionally offer "Reconcile with source" (a second menu entry rather than a checkbox, so each confirm dialog states exactly what will be stamped), and sources MUST NOT.
- **FR-011**: Every user story MUST have an end-to-end test against the scripted fake backend; US1 and US2 MUST additionally be exercised live against the podinfo HelmRelease on the dev cluster before the feature is reported done.

### Key Entities

- **Flux resource**: one of GitRepository, OCIRepository, HelmRepository, Bucket, Kustomization, HelmRelease; identified by kind, namespace and name. Carries `suspended`, `revision`, `ready`, `source_ref` and now `managed` (objects it owns per Flux's inventory).
- **Flux verb**: Suspend, Resume, Reconcile, Reconcile with source.
- **Audit entry**: kind `FluxControl`, naming the resource and the verb.

## Success Criteria *(mandatory)*

- **SC-001**: An operator can suspend any Flux resource from the pane in three interactions: open row, choose verb, confirm.
- **SC-002**: The pane and inspector reflect a suspend, resume or reconcile within one poll interval, and in practice as soon as the backend's post-apply re-read lands.
- **SC-003**: No Flux mutation reaches the cluster without a confirm dialog naming the object; proven by e2e assertions on the sent-command log.
- **SC-004**: Every mutation appears in the audit log with the operator, the resource and the verb.

## Assumptions

- Flux's inventory for a Kustomization lives at `status.inventory.entries[].id` in the form `namespace_name_group_kind`; a HelmRelease's managed objects are not enumerated by Flux, so its dialog names the Helm release it owns (`status.history[0]` or `spec.releaseName`) instead.
- A resource with no inventory lists nothing, and the dialog says so.
- Watching the reconcile to completion is not needed: the post-apply re-read plus the 20s poll already show Reconciling → Ready.
