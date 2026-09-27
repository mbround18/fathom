# Feature Specification: Setup and navigation

**Feature Branch**: `043-setup-and-navigation`

**Created**: 2026-09-27

**Status**: Draft

**Input**: Group 2 "setup and navigation" of `docs/FEEDBACK_2026-09-27.md` (feedback #4, #5, #9/#15, #10, #11, #12, #13/#14, #16, #17, #38), plus #54 (the status bar's left side stays as it is). "A first-run setup screen when no current-context is set, for picking the context, instead of replacing individual panes. Sign-in belongs in setup and/or cluster settings. Cluster settings gets its own section and a header entry left of sign-in, together with team settings. Label and explain the in-cluster daemon toggle; rename or explain 'While you were away'. The UI feels chaotic: group things so someone new to Kubernetes can find their way. Topology leaves the bottom dock: per resource (follow the chain) plus a whole-cluster Topology section. Certificates, Knative, the autoscalers and Velero move into the left-nav Integrations section; greyed 'not installed' entries stay; Velero is hidden unless installed; offering to install integrations is out of scope. The role wizard (042) exists but was not found: make it discoverable from role access. Keep the node view, Tools, the autoscalers, Knative, Hubble, the linter visuals, the context tabs, the certificates panel and the small-window layout."

## Clarifications

### Session 2026-09-27 (self-resolved with defaults; recorded for review)

- Q: Which states get the full setup screen? → A: Only "no usable kubeconfig" (missing, empty, malformed, unreadable) and "contexts exist but none is selected". A selected context that cannot connect keeps today's per-pane state (feedback #1's work), which gains "Choose another context". Reason: once a context is chosen the operator is "in" the app, and the context tabs already show every context's status.
- Q: Does picking a context in setup write `current-context` into the kubeconfig? → A: No. Fathom remembers it in its own UI config (`last_selection`, an existing and unused field) and opens it on the next start when the kubeconfig still selects nothing. A kubeconfig `current-context` always wins over the remembered one. Reason: Principle VI; other tools share the kubeconfig.
- Q: Is sign-in a required setup step? → A: No. It is an optional step with "Skip", offered after a context is picked when OIDC issuers are configured or the connection answered 401. It reuses the existing issuer, sign-in and per-context authentication controls (033) and the exec-plugin status (035). With no issuer it offers "Add an identity provider" and "Skip".
- Q: What does "cluster settings gets its own section" mean concretely? → A: The navigator's "Team" group becomes a "Settings" group with seven pages: Cluster settings (per context only), Sign-in, Team, Connection profiles, App settings (appearance, AI assistant, available agents), Local clusters (kind, desktop only) and Audit log. Today's single Cluster Settings page, which mixes all of these, is split. Existing section keys keep working.
- Q: Where exactly does the header entry go? → A: The sign-in control (identity chip) moves from beside the logo to the right end of the header, and a "Settings" menu button sits immediately to its left, followed only by the overflow menu. The overflow menu's settings items move into the Settings menu. On phone both fold into the overflow menu as today.
- Q: Rename the daemon, or explain it? → A: Keep the name "daemon" (it matches spec 036, the install overlay `k8s/overlays/daemon` and the docs), but the chip always shows the word "Daemon" with its state, the switch is relabelled "Use Fathom's in-cluster daemon" with a description of what on and off do, and the same switch appears in Cluster settings.
- Q: What is "While you were away" renamed to? → A: "Cluster history", with a subtitle: "Changes the in-cluster daemon recorded, including while Fathom was closed." It is also a Cluster › Cluster history navigator entry that explains how to enable it when the daemon is not available.
- Q: What, if anything, stays in the dock? → A: Logs and Events, plus Cluster history only while the daemon is available. These are streams that follow what the operator is doing while they browse, which is what a persistent bottom area is good for. Everything else in the old dock is a destination and moves to the navigator: Topology and Linter to the Cluster group, and Certificates, Knative, KEDA and Velero to Integrations. The dock stays from laptop width up (Principle IV: wider classes keep the persistent tool area).
- Q: What happens to saved layouts whose dock lists the removed tools? → A: They are filtered on load (order, height and open state kept). The saved file is not rewritten until the operator changes the layout, and the `ToolKind` enum is unchanged, so older and newer builds read the same file (Principle VI).
- Q: Does the Tools menu lose the integration entries now that they live in Integrations? → A: No. Tools is on the keep list, so its entries stay as they are, except that Velero follows the "hidden unless installed" rule. The dock is what felt chaotic, not Tools.
- Q: How is Velero "hidden unless installed" squared with "greyed not-installed entries stay"? → A: Velero is the only exception. It is hidden wherever it is listed (navigator, Tools, tablet tab strip, command palette) until detection says it is installed for the active context, and "not yet detected" counts as hidden. Every other integration keeps the greyed "Not installed in this cluster" entry.
- Q: What does "per-resource topology (follow the chain)" include here, and what is left to feedback #23? → A: Here: an inspector "Topology" tab for the kinds the existing related-resources resolver covers, showing one hop up (owned-by, member-of, scaledBy, mounted-by, reconciled-by) and one hop down (owns, selects, scales, scheduled-on), with any other relation listed downstream under its own name, with "Follow" to re-root on a neighbour through the inspector's back history. Left to #23 (Group 4): expanding and collapsing neighbours in place, click-through inside the whole-cluster graph, and large-cluster performance.
- Q: How far does the navigator reorganisation go? → A: Plain group names (Cluster, Workloads, Networking, Storage, Configuration, Scaling, Access Control, Custom Resources, Integrations, Settings). Kind names stay as labels (no renaming of Kubernetes objects, Principle I). Every entry gets a one-line description, and rarely used kinds go under a per-group "More" toggle. Autoscaling built-ins (HPA, VPA, PDB) leave Configuration for a new Scaling group. KEDA stays in Integrations as the user asked.
- Q: Where is the role wizard surfaced? → A: RoleBindings and ClusterRoleBindings toolbars, the "Create a new role…" option in the binding form's and Assign role dialog's role pickers (returning to the binding with the new `roleRef`), the Namespace Access view, and the command palette. All reuse 042's launcher and access gating.
- Q: Does the setup screen conflict with the constitution's "no wizards or onboarding flows"? → A: It is recorded in the plan's Complexity Tracking. The screen names contexts and kubeconfig paths verbatim, hides no Kubernetes object, never mutates anything, and replaces a first-run state that already existed per pane. It is a context picker, not a guided tour.
- Q: What about the status bar? → A: Its left side (connection state, active context, notices) is unchanged (feedback #54). Nothing from this feature is added to its right side, which Group 4 needs for Feedback and Support.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Set up Fathom on a machine with no current context (Priority: P1)

An operator installs Fathom and opens it. Their kubeconfig defines several contexts but no `current-context` (or it has none at all). Instead of every pane showing its own "Pick a context" message, Fathom shows one setup screen: the kubeconfig files it read, every context by name, a filter, "Reload kubeconfig", and (desktop) "Create a local cluster". The operator picks a context, can optionally sign in, and lands in the normal app. Next launch, Fathom opens that context directly.

**Why this priority**: It is the first thing a new user sees. Today it looks broken (the same notice repeated in every pane), which undermines trust before anything else happens.

**Independent Test**: Start with contexts `a`, `b`, `c` and no current context. Assert the setup screen is the only thing in the main area (no panes, no dock, no nav), pick `b`, assert `SelectContext b` is sent, the app shell appears, and the remembered context is saved in Fathom's UI config. Restart with the same kubeconfig and assert `b` opens with no setup screen.

**Acceptance Scenarios**:

1. **Given** contexts exist and none is current, **When** Fathom starts, **Then** the setup screen replaces the whole main area (nav, panes and dock), and the top bar and status bar stay.
2. **Given** no kubeconfig, an empty one, or one that cannot be parsed or opened, **When** Fathom starts, **Then** the setup screen explains the problem in words, lists the paths it looked at, and offers "Reload kubeconfig" and (desktop only) "Create a local cluster". The verbatim error is behind "Details".
3. **Given** more contexts than fit on screen, **When** the operator types in the filter, **Then** the list narrows by substring, and every context stays reachable by keyboard.
4. **Given** the operator picks a context, **When** the connection succeeds, **Then** the setup screen closes and the Overview of that context opens.
5. **Given** the operator picks a context, **When** the connection fails, **Then** the setup screen stays, shows the classified problem (unreachable, 401, 403, TLS, ...) for that context, and offers Retry, "Sign in" (when it applies) and picking another context.
6. **Given** a context was picked in setup, **When** Fathom restarts and the kubeconfig still has no current context, **Then** the remembered context opens directly. Fathom never writes the operator's kubeconfig.
7. **Given** the hosted server build, **Then** no setup screen is ever shown (the context is fixed by the deployment).

---

### User Story 2 - Sign in during setup or from settings (Priority: P1)

A context authenticates with OIDC (or an exec plugin that needs a login). During setup, after picking the context, the operator is offered an optional "Sign in" step: the configured OIDC issuers with their sign-in controls, "Use this identity for <context>", and the context's detected exec plugin with its install hint. Skipping it is always possible. The same controls live in Settings, on a Sign-in page.

**Independent Test**: With one OIDC issuer configured, pick a context in setup, open the sign-in step, sign in (fake device-code flow), bind the context to OIDC, and assert the same commands the identity dialog sends today are emitted. Separately, open Settings › Sign-in and assert the same controls render.

**Acceptance Scenarios**:

1. **Given** setup with a picked context, **When** OIDC issuers are configured or the connection returned 401, **Then** a "Sign in (optional)" step is offered with "Skip".
2. **Given** no issuer is configured, **Then** the step offers "Add an identity provider" (the existing issuer form) and "Skip", never a dead end.
3. **Given** the operator is in the app, **When** they open Settings › Sign-in, **Then** they find issuer settings, per-issuer sign-in and per-context authentication, the same controls as the header sign-in dialog.

---

### User Story 3 - Settings have one home, reachable from the header (Priority: P1)

The operator wants to change this cluster's metrics source. In the header, directly to the left of the sign-in control, there is a "Settings" entry. It opens a menu grouping Cluster settings (for the active context), Sign-in, Team, Connection profiles, App settings, Local clusters and Audit log. The same pages form a "Settings" group at the bottom of the navigator.

**Independent Test**: Click the header Settings entry, choose "Cluster settings", and assert the Cluster settings page for the active context opens in the list pane. Assert the Settings entry sits immediately before the sign-in control in the header's DOM and tab order.

**Acceptance Scenarios**:

1. **Given** any width from laptop up, **Then** the header shows "Settings" (icon and label) immediately left of the sign-in control, both at the right end of the header, before the overflow menu.
2. **Given** a phone, **Then** Settings and its pages are in the overflow menu, as today's settings items are.
3. **Given** the Settings menu, **Then** it lists Cluster settings, Sign-in, Team, Connection profiles, App settings, Local clusters (desktop only) and Audit log, and each opens its page.
4. **Given** Cluster settings, **Then** it holds only per-context settings (metrics source and Prometheus, hidden metrics, org default publish/accept, node shell, cloud auth exec policy, in-cluster daemon). Appearance and AI assistant move to App settings, and local kind clusters to Local clusters.
5. **Given** no cluster is connected, **Then** every Settings page still opens (they are clusterless), and Cluster settings says it needs a context.

---

### User Story 4 - The in-cluster daemon is explained (Priority: P2)

The operator sees a chip reading "Off" with a server icon, and a switch labelled "Use in-cluster daemon if present", and has no idea what either means. After this feature, the chip reads "Daemon: Off" (the word is always there, not just the state), the switch says what it controls, and a short explanation says what the daemon is, what it adds, that it is optional and read-only, and that Fathom never installs it. The dock tab "While you were away" is renamed "Cluster history" with a one-line subtitle explaining it.

**Independent Test**: Open the daemon dialog and assert the switch has an accessible name and description that say what on and off do. With the daemon available, assert the dock tab is named "Cluster history" and its panel starts with the explanation line.

**Acceptance Scenarios**:

1. **Given** the daemon dialog, **Then** the switch is labelled "Use Fathom's in-cluster daemon" with the description "On: Fathom checks this cluster for its optional, read-only daemon and, if found, uses it for cluster history and a faster topology. Off: Fathom never contacts it. Nothing is installed either way."
2. **Given** the daemon is available, **Then** the dock shows a "Cluster history" tab whose panel starts with "Changes the in-cluster daemon recorded, including while Fathom was closed."
3. **Given** the daemon is off, not installed or unavailable, **Then** there is no Cluster history tab in the dock, and the Cluster › Cluster history nav entry explains why it is empty and how to turn it on.
4. **Given** Settings › Cluster settings, **Then** the same switch and explanation are there for the active context.

---

### User Story 5 - A calm dock and a navigator a newcomer can follow (Priority: P1)

The bottom dock today holds nine tabs (logs, events, topology, daemon timeline, Velero, certificates, Knative, KEDA, linter). After this feature it holds only the streams that follow what you are doing: Logs, Events, and Cluster history when the daemon is connected. Everything else is a place you go, and lives in the navigator. The navigator groups things in plain language, gives every entry a one-line description, and tucks rarely used kinds under "More" in their group.

**Independent Test**: On a fresh profile at desktop width, assert the dock has exactly the tabs Logs and Events. Assert the navigator groups and their order match FR-020, every entry has a description, and Velero is absent when not installed. With a saved profile holding the old nine dock tabs, assert the dock shows Logs and Events and the saved file keeps loading.

**Acceptance Scenarios**:

1. **Given** a fresh profile at laptop width or wider, **Then** the dock tabs are Logs and Events, plus Cluster history only while the daemon is available.
2. **Given** a saved layout whose dock lists topology, Velero, certificates, Knative, KEDA or linter, **Then** those tabs are dropped when the layout loads, the rest keep their order, and the user's saved height and open state are kept.
3. **Given** the navigator, **Then** its groups are, in order: Cluster, Workloads, Networking, Storage, Configuration, Scaling, Access Control, Custom Resources, Integrations, Settings.
4. **Given** any navigator entry, **When** hovered or focused, **Then** its one-line description is shown (and read by screen readers), and the command palette shows the same description.
5. **Given** a group with advanced kinds (for example ReplicaSets, Endpoints, Leases, webhook configurations), **Then** they sit under a "More" toggle in that group, collapsed by default, and the palette still finds them.
6. **Given** the operator had groups expanded under the old labels, **When** the app starts, **Then** the matching new groups are expanded.

---

### User Story 6 - Topology: the whole cluster, and the chain around one resource (Priority: P2)

The operator wants to see how the cluster hangs together. "Topology" is now an entry in the Cluster group of the navigator and opens the whole-cluster graph in the main area, with the same view it has today. From any Pod, Deployment, StatefulSet, DaemonSet, ReplicaSet, Service, PVC, Node, Application, Helm release or KEDA scaler open in the inspector, a "Topology" tab shows that resource with its owners, what it owns and what it uses. Clicking a neighbour's "Follow" re-roots the view on it and opens it in the inspector, so the operator can follow the chain from an Ingress down to a Pod's node.

**Independent Test**: Open Deployment `web` in the inspector, choose the Topology tab, assert it requests the related resources of `web`, shows its ReplicaSet and Pods, and that "Follow" on a Pod opens that Pod in the inspector with its own topology (Node, Service, owning ReplicaSet).

**Acceptance Scenarios**:

1. **Given** the navigator, **When** the operator selects Cluster › Topology, **Then** the whole-cluster topology opens in the list pane, including the daemon-fed view when the daemon is available.
2. **Given** a resource whose kind has relationships, **When** its inspector is open, **Then** a "Topology" tab shows the resource in the centre, owners/users above and owned/used resources below, each with kind, name and status hint.
3. **Given** a neighbour, **When** the operator activates "Follow", **Then** the inspector opens that resource (with Back returning to the previous one) and the Topology tab stays selected.
4. **Given** a kind with no relationships, **Then** the Topology tab is not shown.
5. **Given** phone and tablet, **Then** the Topology tab in their tab strips keeps opening the whole-cluster view, as today.

---

### User Story 7 - Integrations live in one place (Priority: P2)

Certificates, Knative, KEDA autoscalers and Velero leave the dock. The navigator's Integrations group is their home, alongside Argo, Flux, Gatekeeper, Hubble, Vulnerabilities, Metrics and Grafana. Integrations that are not installed stay listed, greyed, with "Not installed in this cluster". Velero is the exception: it is not listed at all unless it is installed.

**Independent Test**: With cert-manager installed and Velero, Knative, KEDA absent, assert the Integrations group shows Certificates normally, Knative and KEDA greyed with the hint, and no Velero entry, in the navigator, the Tools menu and the tablet tab strip. Then report Velero installed and assert it appears.

**Acceptance Scenarios**:

1. **Given** Velero is not installed or not yet detected, **Then** Velero appears nowhere (navigator, Tools, tablet tabs, command palette).
2. **Given** Velero is detected as installed, **Then** it appears in Integrations and in Tools.
3. **Given** another integration is not installed, **Then** it stays listed, greyed, with the hint. No entry offers to install anything.

---

### User Story 8 - The role wizard is found from role access (Priority: P2)

The operator is in RoleBindings, wants to grant a team access, and cannot find a role that fits. The role editor from spec 042 is one click away from every place that deals with role access: RoleBindings and ClusterRoleBindings toolbars, the binding form's role picker, the "Assign role" dialog, the Namespace Access view, and the command palette.

**Independent Test**: From the RoleBinding form, choose "Create a new role…", create `pod-reader`, and assert the binding form comes back with `roleRef` Role/pod-reader filled in.

**Acceptance Scenarios**:

1. **Given** the RoleBindings or ClusterRoleBindings pane, **Then** "New Role…" (and "New ClusterRole…") sit beside "New RoleBinding" and "Assign role", with the same access gating as in 042.
2. **Given** the binding form or the Assign role dialog, **When** the operator picks "Create a new role…" in the role picker, **Then** the 042 role form opens, and after a successful create the binding form returns with the new role selected.
3. **Given** the Namespace Access view shows an identity without the access the operator wants, **Then** a "Create a role…" link opens the role form in that namespace.
4. **Given** the command palette, **When** the operator types "role", **Then** "New Role" and "New ClusterRole" actions are offered.

### Edge Cases

- A kubeconfig that sets a `current-context` naming a context that does not exist: treated as no current context (setup screen), as the backend already classifies it.
- A remembered context that was removed from the kubeconfig: ignored, setup screen shown.
- The kubeconfig changes while the setup screen is open ("Reload kubeconfig" or the file watcher): the list updates in place and the filter text is kept.
- The operator switches to another context from the context tabs while the app is running and it fails: per-pane connection states stay as today (feedback #1's work), with a "Choose another context" action; the setup screen is only for "nothing selected".
- The operator opens a Settings page from the header while the setup screen is showing: the page takes the setup screen's place with a "Back to setup" link; any cluster-bound navigation brings the setup screen back.
- Width changes while the setup screen shows: the screen reflows; it has one column on phone.
- Integration detection has not answered yet: non-Velero integrations show as listed (not greyed) until detection says "not installed"; Velero stays hidden.
- A deep link or saved route to a moved section (for example a slot pinned to the old `topology` dock tool): it resolves to the new section; nothing crashes on unknown tool kinds in saved files.
- Hosted mode: the setup screen, Local clusters and kubeconfig-only controls are absent; Settings shows only what hosted supports.

## Requirements *(mandatory)*

### Functional Requirements

**Setup (US1, US2)**

- **FR-001**: When the kubeconfig state is "no usable kubeconfig" or "contexts exist but none is selected", the desktop app MUST show a single setup screen in place of the navigator, panes and dock. The top bar and the status bar stay.
- **FR-002**: The setup screen MUST show the kubeconfig paths read, every context by name with a substring filter, "Reload kubeconfig" with its outcome (feedback #1), "Create a local cluster" (desktop only), and the verbatim error behind "Details".
- **FR-003**: Picking a context MUST send the existing context selection, show connection progress on the setup screen, and leave the setup screen only when the connection succeeds.
- **FR-004**: A failed connection from setup MUST show the classified problem for that context with Retry, "Sign in" (for 401 or when issuers exist) and picking another context.
- **FR-005**: A context picked in setup MUST be remembered in Fathom's own UI config and opened on the next start when the kubeconfig still selects nothing. Fathom MUST NOT write the kubeconfig.
- **FR-006**: The setup screen MUST offer an optional, skippable sign-in step with the existing OIDC issuer, sign-in and per-context authentication controls, and the active context's exec-plugin status.
- **FR-007**: Per-pane first-run states remain only for connection problems of a selected context. They MUST offer "Choose another context".
- **FR-008**: The hosted build MUST NOT show the setup screen.

**Settings (US2, US3)**

- **FR-010**: A "Settings" group MUST replace the "Team" navigator group, with pages: Cluster settings, Sign-in, Team, Connection profiles, App settings, Local clusters (desktop only), Audit log.
- **FR-011**: The header MUST show a Settings entry immediately left of the sign-in control, at the right end of the header, opening a menu of the same pages. On phone these live in the overflow menu.
- **FR-012**: Cluster settings MUST contain only per-context settings, including the in-cluster daemon switch. App-wide settings MUST move to App settings, and local kind clusters to Local clusters.
- **FR-013**: Existing section keys and saved routes for Cluster settings, Teams, Connection profiles and Audit log MUST keep working.

**Daemon (US4)**

- **FR-015**: The daemon chip MUST always show the word "Daemon" with its state at laptop width and wider (icon plus accessible name on narrow), and the switch MUST have an accessible name and description stating what on and off do, that the daemon is optional and read-only, and that Fathom never installs it.
- **FR-016**: "While you were away" MUST be renamed "Cluster history" everywhere (dock tab, pane title, tests, promo), and the panel MUST start with a one-line explanation.

**Dock and navigator (US5)**

- **FR-020**: The navigator groups MUST be, in order: Cluster (Overview, Topology, Nodes, Namespaces, Events, Linter, Cluster history), Workloads (Applications, Helm Releases, Pods, Deployments, StatefulSets, DaemonSets, Jobs, CronJobs; More: ReplicaSets, ReplicationControllers), Networking (Services, Ingress, Network Policies, Port Forwards; More: Endpoints, IngressClasses), Storage (PersistentVolumeClaims, PersistentVolumes, StorageClasses), Configuration (ConfigMaps, Secrets; More: Resource Quotas, Limit Ranges, Priority Classes, Runtime Classes, Leases, Mutating and Validating Webhook Configurations), Scaling (Horizontal Pod Autoscalers, Vertical Pod Autoscalers, Pod Disruption Budgets), Access Control (Roles, RoleBindings, ClusterRoles, ClusterRoleBindings, Namespace Access, OIDC Config), Custom Resources, Integrations, Settings.
- **FR-021**: Every navigator entry MUST have a one-line plain-language description, exposed as a tooltip, as the entry's accessible description, and in the command palette. Kubernetes kind names MUST stay as the labels.
- **FR-022**: "More" toggles MUST be collapsed by default, persisted like group expansion, and MUST NOT hide entries from search.
- **FR-023**: Persisted expanded groups under old labels MUST map to the new groups (Overview → Cluster, Network → Networking, Config → Configuration and Scaling, Team → Settings).
- **FR-024**: The default dock MUST hold Logs and Events. Cluster history MUST be added while the daemon is available for the active context, and removed otherwise.
- **FR-025**: The dock MUST accept only Logs, Events and Cluster history. Other tools in a saved dock MUST be dropped on load, keeping order, height and open state, without rewriting or deleting the saved file until the user next changes the layout.
- **FR-026**: The status bar's left side (connection state and active context) MUST stay unchanged (feedback #54). Its right side stays free for Group 4's Feedback and Support buttons.

**Topology (US6)**

- **FR-030**: "Topology" MUST be a navigator section in the Cluster group, rendering today's whole-cluster topology (client-built or daemon-fed) in a list pane.
- **FR-031**: The inspector MUST offer a "Topology" tab for kinds the related-resources resolver covers, showing the resource, its upstream (owned-by, member-of, scaledBy, mounted-by, reconciled-by) and downstream (owns, selects, scales, scheduled-on; any other relation, under its own name) neighbours from the existing read-only related-resources data.
- **FR-032**: Each neighbour MUST offer "Follow", which opens it in the inspector with back history and keeps the Topology tab selected.
- **FR-033**: Topology MUST remain in the Tools menu and in the phone and tablet tab strips.

**Integrations (US7)**

- **FR-035**: Certificates, Knative, KEDA and Velero MUST NOT be dock tabs. Their home is the Integrations group; their panels are unchanged.
- **FR-036**: Velero MUST be hidden in the navigator, Tools, tablet tabs and command palette unless it is detected as installed for the active context.
- **FR-037**: Other integrations that are not installed MUST stay listed and greyed with "Not installed in this cluster". No entry may offer to install an integration.

**Role discoverability (US8)**

- **FR-040**: "New Role…" and "New ClusterRole…" MUST be offered in the RoleBindings and ClusterRoleBindings toolbars, in the binding form's and Assign role dialog's role pickers as "Create a new role…", in the Namespace Access view, and in the command palette, using 042's launcher and access gating.
- **FR-041**: A role created from a binding flow MUST return to that flow with the new role as `roleRef`. Nothing is created until the operator applies each form.

**Keep list (all stories)**

- **FR-045**: The node view, Tools menu and sheet, KEDA and HPA/VPA views, the Knative panel, the Hubble integration, the linter's visuals, the context tabs, the certificates panel, and the phone and tablet layouts MUST keep their current behaviour, apart from the moves and labels named above.

### Key Entities

- **SetupState**: which setup view applies (`kubeconfig` issue, `pick`, `connecting <context>`, `failed <context, problem>`, `sign-in <context>`), derived from existing session state.
- **RememberedContext**: the context picked in setup, stored in Fathom's UI config (`last_selection`), never in the kubeconfig.
- **NavModel**: groups, each with entries (section key, label, description, advanced flag, visibility rule), plus the legacy-label map.
- **DockPolicy**: the allowed dock tools, the default tabs, and the filter applied to saved docks.
- **ResourceTopology**: a root resource plus its related resources split into upstream and downstream by relation.

## Success Criteria *(mandatory)*

- **SC-001**: With no current context, 100% of first starts show exactly one setup screen and no per-pane "Pick a context" messages.
- **SC-002**: A new user can go from launch to a connected context's Overview in at most 3 actions (pick, optional skip, done).
- **SC-003**: Cluster settings is reachable in at most 2 clicks from anywhere at laptop width and wider.
- **SC-004**: The default dock has at most 3 tabs, down from 9.
- **SC-005**: Every navigator entry (100%) has a description, and at most 8 entries are visible before "More" in every group except Custom Resources and Integrations, whose lists depend on what the cluster has installed.
- **SC-006**: "New Role" is reachable in at most 2 clicks from RoleBindings, ClusterRoleBindings, the binding form and Namespace Access.
- **SC-007**: No item on the keep list loses behaviour: every existing E2E test covering those views passes, updated only for moved entry points and renamed labels.
- **SC-008**: Saved layouts and UI configs from before this feature load with no manual steps and no errors.

## Assumptions

- The first-run classification (`KubeconfigStatus`, `ActiveConnectionProblem`, `useFirstRun`) and the reload outcome from feedback #1 are reused unchanged; the setup screen is a new presentation of them.
- The related-resources resolver (`RefreshRelatedResources`, spec 004) already covers the kinds listed in US6 and needs no backend change; wider relationships and expand/collapse belong to feedback #23 (Group 4).
- `UiConfig.last_selection` exists in the protocol and is unused today; using it needs no protocol change.
- Section keys are strings; new UI-only sections (`topology`, `linter`, `cluster-history`, `settings/sign-in`, `settings/app`, `settings/local-clusters`) need no backend route handling.
- Group 3 (cascading slots) will change `PROFILES` and slot assignment; this feature changes only the dock and the nav/section structure, which Group 3 consumes.
- Out of scope: installing integrations, the cascading slot model (Group 3), the logs viewer, the fleet dashboard, pop-out windows, Feedback/Support buttons (Group 4), topology flyout improvements (#23).
