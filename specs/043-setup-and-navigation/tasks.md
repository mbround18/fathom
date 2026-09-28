# Tasks: Setup and navigation

Everything is UI-only (`ui/`). There are no protocol or backend changes. `[P]` marks tasks that touch different files and can run in parallel. Test tasks come first in each story and must fail before the implementation.

## Phase 1: Foundations (pure logic, no UI)

- [x] T001 [P] Add `setupStateOf()` and the `SetupState` union (kinds `kubeconfig`, `pick`, `connecting`, `failed`, `sign-in`; `hosted` always gives `undefined`; `connecting`/`failed` only when `pickedInSetup`) in `ui/src/components/setup/setup-state.ts`, built on `firstRunOf` from `ui/src/lib/first-run.ts`
- [x] T002 [P] Unit tests for every transition in data-model.md, including hosted, the missing current-context, and a connection problem with no setup pick, in `ui/tests/unit/setupState.test.ts`
- [x] T003 [P] Add `DOCK_ALLOWED = ['logs','events','daemon']`, the default dock `['logs','events']`, `filterDock()` (keeps order, `open`, `heightPct`) and `dockTabs(dock, daemonAvailable)` in `ui/src/layout/defaults.ts`, and apply `filterDock` inside `resolveProfile`. Do not touch `PROFILES` (Group 3)
- [x] T004 [P] Dock-policy unit tests: the old nine-tab dock filters to logs/events, the daemon tab is added or removed with availability, and saved height and open state are kept, in `ui/tests/unit/layout-defaults.test.ts`
- [x] T005 [P] Add `splitRelated(root, refs)` → `{upstream, downstream}` in `ui/src/topology/resource-topology.ts`, with `UPSTREAM = {'owned-by','member-of','scaledBy','mounted-by','reconciled-by'}`, all other relations downstream and labelled, and `RESOLVER_KINDS` as listed in data-model.md
- [x] T006 [P] Unit tests for `splitRelated`, covering an unknown relation, an empty result and a Service/PVC/Pod root, in `ui/tests/unit/resourceTopology.test.ts`
- [x] T007 Rewrite `ui/src/resources/nav.ts` as a `NAV_MODEL`:
  - Groups and order exactly as spec FR-020.
  - Each entry has a `description` (at most 90 characters, plain language, not repeating the label).
  - `advanced` flags as FR-020 "More" lists.
  - `LEGACY_GROUP_LABELS` (Overview→Cluster, Network→Networking, Config→Configuration+Scaling, Team→Settings).
  - `integrationVisible(key, installed)` (Velero only when `installed === true`).
  - New sections: `topology`, `linter`, `cluster-history`, `settings/sign-in`, `settings/app`, `settings/local-clusters`.

  Keep the `PANEL_SECTIONS` and `NAV_TOOLS` exports working for existing callers.
- [x] T008 Rewrite `ui/tests/unit/navModel.test.ts` and update `ui/tests/unit/navCoverage.test.ts` and `ui/tests/unit/paletteIndex.test.ts`. They cover:
  - group order
  - every entry has a description
  - at most 8 non-advanced entries per group, except Custom Resources and Integrations (SC-005)
  - legacy label mapping
  - every previous section is still reachable

  Add `ui/tests/unit/veleroVisibility.test.ts` for the unknown/false/true cases.

## Phase 2: US1 setup screen (P1) 🎯 MVP

- [x] T010 [US1] E2E `ui/tests/e2e/setup-screen.spec.ts`. It covers:
  - pick-state: only `setup-screen` is in the main area; the top bar and status bar are present.
  - the filter.
  - pick → `SelectContext` + `SaveUiConfig.lastSelection`.
  - a failed connect keeps the screen with Retry and "choose another".
  - a missing, empty or malformed kubeconfig copy, with Details.
  - a restart with a remembered context skips setup.
  - hosted mode never shows it.
  - phone width uses one column.
  - a kubeconfig reload keeps the filter text.
  - a remembered context that was removed shows setup.
  - a Settings page opened from the header replaces setup, with "Back to setup".
- [x] T011 [P] [US1] Add `SetupScreen.tsx` in `ui/src/components/setup/`. It contains:
  - the kubeconfig paths
  - a `listbox` named "Contexts" (virtualised above 200 entries)
  - a "Filter contexts" searchbox
  - "Reload kubeconfig" with its `reloadOutcome`
  - "Create a local cluster" (desktop only, opens the existing kind flow)
  - verbatim "Details"
  - connecting and failed views using `connectionProblemCopy`
- [x] T012 [US1] In `ui/src/components/shell/AppShell.tsx`, render `SetupScreen` in place of the navigator, `SlotGrid`/`PhoneStack` and the dock when `setupStateOf` returns a state. Keep `TopBar` and `StatusBar` untouched. A clusterless Settings page opened from the header while setup shows takes the setup screen's place with a "Back to setup" link, and cluster-bound navigation returns to setup
- [x] T013 [US1] Store: `rememberContext(ctx)` writes `lastSelection {context, section:'overview'}` through the existing `SaveUiConfig` path. `startupContext()` selects the remembered context only when the kubeconfig has no valid current-context and the name still exists. Both go in `ui/src/store/index.ts`, with unit tests in `ui/tests/unit/rememberedContext.test.ts`
- [x] T014 [US1] In `ui/src/components/shell/FirstRunState.tsx`, keep only the `connection` kind in panes and add "Choose another context", which opens the context picker. Update `ui/tests/unit/first-run.test.tsx` and `ui/tests/e2e/first-run-states.spec.ts` so the kubeconfig and pick cases assert the setup screen

## Phase 3: US2 sign-in in setup and settings (P1)

- [x] T020 [US2] E2E `ui/tests/e2e/setup-sign-in.spec.ts`. It covers:
  - the step is offered when issuers exist or on a 401
  - Skip
  - a fake device-code sign-in
  - binding a context to OIDC emits the same commands as the IdentityChip dialog
  - with no issuer: "Add an identity provider" and Skip
  - Settings › Sign-in renders the same controls
- [x] T021 [P] [US2] Add `SignInSettings.tsx` in `ui/src/features/identity/`. It composes `IssuerSettings`, `SignInPanel`, `ContextAuth` and the exec-plugin status, and is used by both setup and Settings
- [x] T022 [US2] Add `SetupSignIn.tsx` in `ui/src/components/setup/`, with the heading "Sign in (optional)" and a Skip button, wired into `SetupScreen` after a pick
- [x] T023 [US2] Map the `settings/sign-in` section to `SignInSettings` in `ui/src/components/shell/PaneHost.tsx` (clusterless)

## Phase 4: US3 settings home and header entry (P1)

- [x] T030 [US3] E2E `ui/tests/e2e/settings-home.spec.ts`. It covers:
  - the header Settings button is the DOM sibling immediately before the identity chip, at the right, before the overflow menu
  - the menu items and each page opening
  - on phone, the items are in the overflow menu
  - every Settings page opens with no cluster, and Cluster settings says it needs a context
  - Local clusters is absent in hosted mode
- [x] T031 [P] [US3] Add `SettingsMenu.tsx` in `ui/src/components/settings/`, with Cluster settings, Sign-in, Team, Connection profiles, App settings, Local clusters (desktop, not hosted) and Audit log, each calling `navigate(section)`
- [x] T032 [US3] In `ui/src/components/shell/TopBar.tsx`, move `IdentityChip` into the right action cluster and put `SettingsMenu` immediately before it. In `ui/src/components/shell/OverflowMenu.tsx`, drop Settings, Audit and Profiles at laptop width and wider and keep them on phone
- [x] T033 [P] [US3] Split `ui/src/components/panels/ClusterSettingsPanel.tsx`:
  - It keeps the per-context settings (metrics source, Prometheus, hidden metrics, org default publish/accept, node shell, cloud auth exec policy).
  - New `ui/src/components/settings/AppSettingsPanel.tsx` gets appearance, AI assist and available agents.
  - New `ui/src/components/settings/LocalClustersPanel.tsx` gets `KindClusterPanel`.
- [x] T034 [US3] Map `settings/app` and `settings/local-clusters` in `ui/src/components/shell/PaneHost.tsx`, add them to `CLUSTERLESS_SECTIONS`, and keep `integrations/cluster-settings`, `team/teams`, `team/profiles` and `audit-log` working (FR-013)
- [x] T035 [US3] Update `ui/tests/e2e/us2-teams-cluster-settings.spec.ts`, `ui/tests/e2e/us6-settings.spec.ts`, `ui/tests/e2e/observability-settings.spec.ts`, `ui/tests/e2e/mcp-settings-persistence.spec.ts`, `ui/tests/e2e/us1-oidc-identity.spec.ts` and `ui/tests/e2e/batch-i-ai-identity-portforward.spec.ts` for the new entry points and moved items, keeping every assertion. Update the related unit tests (`ui/tests/unit/aiAssist.test.tsx`, `ui/tests/unit/availableAgentsSection.test.tsx`, `ui/tests/unit/observabilitySettings.test.tsx`)

## Phase 5: US5 calm dock and newcomer navigator (P1)

- [x] T040 [US5] E2E `ui/tests/e2e/dock-and-nav.spec.ts`. It covers:
  - on a fresh profile at desktop width, the dock is exactly Logs and Events
  - with the daemon available, Cluster history is added
  - a saved profile with the nine old tabs loads with Logs and Events, keeps its height and emits no `SaveUiConfig` until the user changes the layout
  - the group order
  - each entry's tooltip and `aria-describedby`
  - "More" is collapsed by default and persisted, and the palette still finds advanced kinds
  - old expanded labels map to the new groups
- [x] T041 [US5] In `ui/src/components/shell/SlotGrid.tsx`, render the dock tabs from `dockTabs(profile.dock, daemonAvailable)`. In `ui/src/store/index.ts` (`assignPanes`), create `dock-${tool}` panes only for allowed tools
- [x] T042 [US5] In `ui/src/components/NavTree.tsx`:
  - render `NAV_MODEL`: groups, descriptions as tooltip plus `aria-describedby`
  - add "More <group>" toggles (`aria-expanded`), persisted as `"<Group>/More"` in `expanded_groups`
  - map legacy labels on load
  - keep the Tools list
- [x] T043 [P] [US5] Show the entry descriptions in `ui/src/components/CommandPalette.tsx`, and index advanced entries
- [x] T044 [US5] Update `ui/tests/e2e/nav-walk.spec.ts`, `ui/tests/e2e/shell.spec.ts`, `ui/tests/e2e/responsive-shell.spec.ts`, `ui/tests/e2e/a11y-shell.spec.ts` and `ui/tests/e2e/us1-command-palette.spec.ts` for the new group names, dock tabs and header order
- [x] T045 [US5] Assert the status bar's left side is unchanged (connection state, context) at every width class, with no new right-side items, in `ui/tests/e2e/dock-and-nav.spec.ts` (FR-026)

## Phase 6: US4 daemon explained (P2)

- [x] T050 [US4] E2E `ui/tests/e2e/daemon-explained.spec.ts`. It covers:
  - the chip text "Daemon: <state>" at laptop width, and its accessible name at phone width
  - the switch name "Use Fathom's in-cluster daemon" and its on/off description (spec US4 scenario 1, verbatim)
  - the dock tab "Cluster history" and its subtitle
  - the Cluster › Cluster history explanation when the daemon is off
  - the same switch in Cluster settings
- [x] T051 [P] [US4] Add the `DAEMON_SWITCH_LABEL`/`DAEMON_SWITCH_DESCRIPTION`/`CLUSTER_HISTORY_SUBTITLE` copy to `ui/src/features/daemon/state.ts`. Add a label and `aria-describedby` in `ui/src/features/daemon/DaemonChip.tsx`, and extract a `DaemonSwitch` component reused by `ClusterSettingsPanel.tsx`
- [x] T052 [US4] In `ui/src/components/shell/labels.ts`, set `TOOL_LABEL.daemon = 'Cluster history'`. In `ui/src/features/daemon/DaemonTimelinePanel.tsx`, add the subtitle and the not-available explanation. Map the `cluster-history` section in `PaneHost.tsx`
- [x] T053 [US4] Update `ui/tests/e2e/us1-daemon.spec.ts` and `ui/tests/unit/daemon-store.test.ts` for the new label

## Phase 7: US6 topology sections (P2)

- [x] T060 [US6] E2E `ui/tests/e2e/topology-sections.spec.ts`. It covers:
  - Cluster › Topology opens the whole-cluster graph, and the daemon view when the daemon is available
  - the Deployment inspector Topology tab sends `RefreshRelatedResources` and shows the ReplicaSet and Pods
  - Follow on a Pod opens it with Back and the tab kept
  - no tab for a ConfigMap
  - the phone and tablet Topology tabs, and the Tools menu, still open the whole view (FR-033)
  - a saved layout pinned to the old `topology` dock tool loads without error and opens the section
- [x] T061 [P] [US6] Add `TopologyTab.tsx` in `ui/src/components/inspector/`. It has Upstream and Downstream sections, each neighbour shows kind, name, status hint and relation label, and "Follow" calls `openInspector(ref, from, true)` with the tab kept. It discards events for a superseded root
- [x] T062 [US6] Add the tab in `ui/src/components/Inspector.tsx`, gated by `RESOLVER_KINDS`, and persist the selected inspector tab across Follow in `ui/src/store/index.ts`
- [x] T063 [US6] Map the `topology` section to `TopologyPanel` and `linter` to the linter panel in `ui/src/components/shell/PaneHost.tsx`. In `ui/src/store/index.ts`, resolve a persisted slot or route pinned to an old dock tool (`topology`, `linter`, `velero`, `certManager`, `knative`, `keda`) to its section, and skip unknown tool kinds without throwing (spec edge case "deep link or saved route")
- [x] T064 [US6] Update `ui/tests/e2e/us1-topology-overview.spec.ts`, `ui/tests/e2e/us2-topology-zoom.spec.ts`, `ui/tests/e2e/us3-topology-inspector.spec.ts` and `ui/tests/e2e/us4-topology-live-update.spec.ts` to open Topology from the navigator, keeping every graph assertion

## Phase 8: US7 integrations home (P2)

- [x] T070 [US7] E2E `ui/tests/e2e/integrations-home.spec.ts`. With cert-manager installed and Velero/Knative/KEDA absent, Certificates is normal, Knative and KEDA are greyed with "Not installed in this cluster", and Velero is absent from the navigator, Tools, tablet tabs and palette. After Velero is detected, it appears. No install offer exists anywhere
- [x] T071 [US7] Apply `integrationVisible` in `ui/src/components/NavTree.tsx`, in the Tools menu in `ui/src/components/shell/TopBar.tsx`, in `ui/src/components/shell/ToolsSheet.tsx`, in the tablet tabs in `ui/src/components/shell/stack-tabs.ts` (VELERO_TAB) and in the palette index
- [x] T072 [US7] Update `ui/tests/e2e/nav-integrations.spec.ts` and `ui/tests/e2e/velero-pane.spec.ts` so Velero is marked installed before asserting its pane

## Phase 9: US8 role wizard discoverable (P2)

- [x] T080 [US8] E2E `ui/tests/e2e/role-discoverability.spec.ts`. It covers:
  - "New Role…" and "New ClusterRole…" in the RoleBindings and ClusterRoleBindings toolbars, with 042 gating
  - "Create a new role…" in the RoleBinding form picker → create `pod-reader` → the form returns with `roleRef` Role/pod-reader
  - the same flow from Assign role
  - the Namespace Access "Create a role…" link
  - the palette "role" query
- [x] T081 [US8] Add `RoleCreateLauncher` to the `access/rolebindings` and `access/clusterrolebindings` toolbars in `ui/src/components/shell/PaneHost.tsx`
- [x] T082 [US8] Add a "Create a new role…" picker option with an `onCreated(roleRef)` return in `ui/src/components/rbac/RoleBindingForm.tsx` and `ui/src/features/roles/RoleAssignmentLauncher.tsx`. Add an `onCreated` prop to `ui/src/components/rbac/RoleForm.tsx` / `RoleCreateLauncher.tsx`
- [x] T083 [P] [US8] Add the "Create a role…" link in `ui/src/resources/namespaceAccess.tsx`, and the "New Role"/"New ClusterRole" palette actions in `ui/src/components/CommandPalette.tsx`
- [x] T084 [US8] Extend `ui/tests/unit/rolebindingForm.test.ts` and `ui/tests/unit/roleAssignment.test.tsx` for the picker option

## Phase 10: Polish and keep-list

- [x] T090 Keep-list regression run: node view (`node-shell.spec.ts`), Tools, KEDA/HPA/VPA (`ui/tests/unit/keda-panel.test.tsx`), Knative (`ui/tests/unit/knative-panel.test.tsx`), Hubble (`nav-integrations.spec.ts`), linter (`ui/tests/unit/linter`), context tabs, certificates (`ui/tests/unit/certManagerPanel.test.tsx`), phone/tablet layouts (`responsive-shell.spec.ts`). Any failure is fixed in code, not by loosening the test
- [x] T091 Update `docs/MVP.md` (the Cluster Settings section and the navigation/dock descriptions) for the setup screen, the Settings group and header menu, the dock contents, Cluster history and the navigator groups
- [x] T092 Tick the feedback items #4, #5, #9/#15, #10, #11, #12, #13/#14, #16, #17, #38 and #54 as addressed by 043 in `docs/FEEDBACK_2026-09-27.md`
- [x] T093 Brochure/promo (last): rename 'While you were away' → 'Cluster history' in `ui/tests/promo/compose.spec.ts`, and capture the setup screen, Settings menu, calm dock and the inspector Topology tab in `ui/tests/promo/capture.spec.ts`

## Dependencies

- Phase 1 blocks everything. T001 blocks US1 and US2, T003 blocks US5/US4/US6, T005 blocks US6, and T007 blocks US3, US5, US6 and US7.
- US1 → US2 (the sign-in step sits inside the setup screen). US3 is independent of US1.
- US4, US6, US7 and US8 depend only on Phase 1 and can run in parallel after it. US4 and US6 touch `PaneHost.tsx`, so land them sequentially.
- T093 runs after every story is done.

## Parallel examples

- Phase 1: T001, T003, T005 and T007 in parallel (four files), then the tests T002, T004, T006 and T008.
- After Phase 1: one agent on US1→US2, one on US3, one on US5, one on US7+US8. Then US4 and US6.

## Implementation strategy

MVP is Phase 1 plus US1 (the setup screen), which directly answers the first-launch complaint. Then US3 and US5 (Settings home and the calm dock/nav), which carry most of the "chaotic" feedback, followed by the P2 stories. Use `--workers=6` and `CARGO_BUILD_JOBS=4` when other agents are active.

## Follow-ups (not in 043)

- [ ] Topology expand/collapse in place and graph click-through (feedback #23, Group 4)
- [ ] Cascading slots, and where new sections land at each width (Group 3)
- [ ] Feedback and Support buttons on the status bar's right side (Group 4, #52/#53)
- [ ] Offering to install integrations (post-MVP)
