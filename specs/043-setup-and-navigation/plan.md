# Implementation Plan: Setup and navigation

**Branch**: `043-setup-and-navigation` | **Date**: 2026-09-27 | **Spec**: [spec.md](spec.md)

## Summary

This feature reorganises how the app is entered and navigated. It adds no new cluster capability.

- **Setup screen.** A single setup screen replaces the per-pane "Pick a context" and "no kubeconfig" states. It reuses `firstRunOf`/`useFirstRun` and the feedback #1 reload outcome.
- **Remembered context.** A context picked in setup is remembered in `UiConfig.last_selection`, which already exists and is unused.
- **Settings.** Settings become one navigator group and one header menu, placed left of the sign-in chip, which moves to the right. Today's Cluster Settings page is split into per-context, sign-in, app and local-cluster pages.
- **Dock.** The dock is reduced to streams: Logs, Events and Cluster history. Saved docks are filtered on load.
- **Moved destinations.** Topology, Linter and Cluster history become navigator sections.
- **Per-resource topology.** A per-resource Topology tab in the inspector renders the existing related-resources data as upstream and downstream, with "Follow".
- **Navigator rewrite.** `nav.ts` is rewritten as a described, grouped model with "More" toggles.
- **Velero.** Hidden unless it is installed.
- **Role create.** The 042 role launcher gets entry points from every role-access surface.

Everything is UI-only. There are no protocol changes and no backend changes.

## Technical Context

- **Backend**: none. Reused as-is:
  - `KubeconfigStatus` / `ActiveConnectionProblem` (first run)
  - `SelectContext`
  - `SaveUiConfig` with `last_selection` (`src/protocol/v1/ui_config.rs`, `LastSelection {context, namespace?, section}`)
  - `RefreshRelatedResources` / `RelatedResourcesUpdated` (spec 004)
  - integration detection
  - daemon preference commands (036)
  - OIDC issuer and context-auth commands (033)
- **UI (React/TS, Zustand)**:
  - `ui/src/components/setup/`:
    - `SetupScreen.tsx`: context list, filter, reload, local cluster, Details.
    - `SetupSignIn.tsx`: the optional step around `SignInSettings` (below).
    - `setup-state.ts`: pure `setupStateOf()`.
  - `AppShell.tsx`: renders `SetupScreen` in place of nav, `SlotGrid`/`PhoneStack` and the dock when `setupStateOf` says so. `FirstRunState.tsx` keeps only the connection-problem kind, plus "Choose another context".
  - `ui/src/resources/nav.ts`: a new `NAV_MODEL` (groups → entries with `description`, `advanced`, `visible(ctx)`), plus `LEGACY_GROUP_LABELS`. `NavTree.tsx` renders the "More" toggles, tooltips and `aria-describedby`. `CommandPalette.tsx` shows the descriptions.
  - `ui/src/layout/defaults.ts`: `DOCK_ALLOWED = ['logs','events','daemon']`, `DOCK` default `['logs','events']`, and `filterDock()` used by `resolveProfile`. The daemon tab is appended at render time while the daemon is available (`SlotGrid.tsx`).
  - Settings:
    - `ui/src/components/settings/` holds `SettingsMenu.tsx` (header), `AppSettingsPanel.tsx` and `LocalClustersPanel.tsx`.
    - `ClusterSettingsPanel.tsx` is trimmed to per-context settings, plus the `DaemonSwitch` section.
    - `ui/src/features/identity/SignInSettings.tsx`: composes `IssuerSettings`, `SignInPanel`, `ContextAuth` and the exec-plugin status. Used by setup and by Settings › Sign-in.
  - `TopBar.tsx`: `IdentityChip` moves to the right cluster, and `SettingsMenu` sits immediately before it. `OverflowMenu.tsx` loses Settings/Audit/Profiles on laptop and wider and keeps them on phone.
  - Daemon (`ui/src/features/daemon/`): `DaemonChip.tsx` gets the label, switch copy and description. `state.ts` gets `DAEMON_SWITCH_*` copy. `labels.ts`: `TOOL_LABEL.daemon = 'Cluster history'`. `DaemonTimelinePanel.tsx` gets the subtitle and the not-available explanation.
  - Topology:
    - New `ui/src/topology/resource-topology.ts`: pure `splitRelated(root, refs)` → `{upstream, downstream}`.
    - New `ui/src/components/inspector/TopologyTab.tsx`, and an `Inspector.tsx` tab gated by `RESOLVER_KINDS`.
    - The `topology` section maps to `TopologyPanel` in `PaneHost.tsx`, as do `linter` and `cluster-history`.
  - Velero: `integrationVisible(key, installed)` in `nav.ts`. It is used by `NavTree`, `ToolsSheet`/`TopBar` ToolsMenu, `stack-tabs.ts` (tablet) and the palette index.
  - Roles:
    - `PaneHost.tsx` adds `RoleCreateLauncher` to the `access/rolebindings` and `access/clusterrolebindings` toolbars.
    - `RoleBindingForm.tsx` and `features/roles/RoleAssignmentLauncher.tsx` get a "Create a new role…" picker option with an `onCreated(roleRef)` return.
    - `resources/namespaceAccess.tsx` gets a "Create a role…" link.
    - The palette gets actions.
- **Store** (`ui/src/store/index.ts`):
  - `rememberContext(ctx)`: writes `last_selection` through the existing `SaveUiConfig` path.
  - `startupContext()`: picks the remembered context when the kubeconfig has no current context.
  - `nav.moreExpanded`: persisted beside `expanded_groups` as `"<Group>/More"` labels.
  - No change to `assignPanes`, apart from the dock pane set following `filterDock`.
- **Testing**:
  - Vitest:
    - `setupState.test.ts`, `navModel.test.ts` (rewritten), `layout-defaults.test.ts` (dock policy)
    - `resourceTopology.test.ts`, `rememberedContext.test.ts`
    - `veleroVisibility.test.ts`
  - Playwright mock: a new spec per story (`setup-screen`, `setup-sign-in`, `settings-home`, `daemon-explained`, `dock-and-nav`, `topology-sections`, `integrations-home`, `role-discoverability`), plus updates to the existing shell/nav/topology/daemon specs listed in research.md.
  - Promo: `compose.spec.ts` label change and new captures, done last.

## Constitution Check

- **I. Operator-first, explicit mutation.** Nothing in this feature mutates the cluster. The role flows still go through 042's review and confirm, and a binding is not created until the operator applies it. Errors stay verbatim behind "Details". The setup screen is the one tension with "wizards, onboarding flows… out of scope"; see Complexity Tracking. ✔ (justified)
- **II. One contract.** No protocol change. `last_selection` is written only through `SaveUiConfig`. The UI never touches `~/.fathom/` or the kubeconfig. ✔
- **III. Tested at the boundaries.**
  - Every story has a mock E2E spec.
  - Everything that decides what the operator sees has unit tests: the setup state, nav model, dock policy, topology split and Velero visibility.
  - Existing tests updated for moved entry points keep their assertions. Coverage that moves (for example, the dock Topology tab) is replaced by the nav or Tools path. ✔
- **IV. Density scales with width.**
  - The dock stays at laptop width and wider, so the persistent tool area is still present.
  - Pane counts per width class are unchanged, because PROFILES are untouched.
  - The setup screen reflows to one column on phone.
  - The header Settings menu and the "More" toggles are keyboard- and touch-operable.
  - Descriptions are exposed with `aria-describedby`. ✔
- **V. Fast and quiet.**
  - The nav model is static data.
  - The topology tab reuses the resolver's single request per root and discards events for a superseded root.
  - The setup screen virtualises context lists longer than 200 entries. ✔
- **VI. User data is sacred.**
  - Saved docks are filtered in memory, and the file is rewritten only on the user's next layout change.
  - The `ToolKind` enum is unchanged.
  - Old `expanded_groups` labels are mapped.
  - The kubeconfig is never written. ✔
- **Hosted (constitution platform section).** There is no setup screen in hosted mode, and Local clusters is hidden there. ✔

## Complexity Tracking

| Tension | Why needed | Simpler alternative rejected because |
|---|---|---|
| Principle I lists "onboarding flows" and "wizards" as out of scope, and this feature adds a setup screen with an optional sign-in step. | The per-pane first-run state repeats the same "pick a context" message in every pane (feedback #4), which reads as broken. One screen in the main area is the honest presentation of a single app-wide state. It shows contexts and kubeconfig paths verbatim, hides no Kubernetes object, performs no mutation and can always be left (Skip / pick). It is a context picker, not a guided tour. | Keeping per-pane states: this is the complaint itself. A modal picker over the panes blocks the status bar and top bar, which must stay usable (feedback #1 reload, #54 status bar). |
| Principle I, "abstractions that hide Kubernetes objects": the navigator gets plain-language group names and descriptions. | Feedback #13/#14 asks for a layout that someone new to Kubernetes can follow. | Renaming kinds was rejected. Kind names stay as the labels, and descriptions are additive (tooltip, a11y description, palette). |
| Principle IV, "each wider class adds persistent tool areas": the default dock shrinks from 9 tabs to 2 or 3. | Feedback #10, #11 and #16 describe the dock as chaotic. The removed tabs become full navigator destinations, reachable at every width, and still open from Tools. | Removing the dock entirely would reduce what wider classes show and would break Principle IV. The dock remains from laptop width up. |

## Design decisions

- **The setup screen is an `AppShell` branch, not a pane.** It replaces the navigator, slot grid and dock when `setupStateOf(kubeconfig, activeContext, hosted)` returns a state. This is independent of the slot model, so Group 3's cascading slots do not interact with it.
- **Connection failures stay per-pane.** Once a context is selected, the context tabs already carry per-context status. The setup screen is only for "nothing selected" or "nothing usable".
- **Remembered context in `last_selection`.** `LastSelection.section` is set to `overview`. A kubeconfig `current-context` always wins. A remembered context that no longer exists is ignored.
- **Dock = streams.** Logs and Events follow what the operator is looking at, and Cluster history is a live feed. Every other old dock tab is a destination: a whole-cluster graph, a lint report, or an integration inventory. Destinations belong in the navigator, where they have a name, a description and a pane of their own.
- **Filter, don't migrate.** `filterDock` runs in `resolveProfile`, and the dock `ToolKind` values stay valid in the enum. An older build reading a newer file sees a subset it understands. A newer build reading an older file drops the extra tabs in memory.
- **Topology tab reuses the resolver.** The resolver's `relation` describes the root's link to the neighbour (`src/backend/related.rs`, `helm.rs`): `owned-by`, `member-of`, `scaledBy`, `mounted-by` and `reconciled-by` go up; `owns`, `selects`, `scales`, `scheduled-on` and any unknown relation go down, labelled with the relation name. "Follow" is `openInspector(ref, from, push=true)` with the inspector's `tab` kept as `topology`. Expand/collapse and graph click-through remain with #23.
- **Nav model as data.** One `NAV_MODEL` feeds the tree, the palette index and the coverage test (`navCoverage.test.ts`). The "More" state reuses the persisted `expanded_groups` list with `"<Group>/More"` entries, so no protocol field is added.
- **Velero is the only hidden integration.** The user asked for this explicitly. `unknown` counts as hidden, so it never flickers into view.
- **Status bar untouched.** This feature makes no change to `StatusBar.tsx`. Group 4 adds its buttons on the right side.

## Keep-list review

| Item | Where it ends up | Status |
|---|---|---|
| Node view | Cluster › Nodes; inspector unchanged | kept |
| Tools (menu + sheet) | unchanged, except Velero hidden unless installed | kept |
| Autoscalers (KEDA, HPA/VPA) | KEDA in Integrations and Tools; HPA/VPA/PDB in the new Scaling group; panels unchanged | kept (moved) |
| Knative | Integrations and Tools; panel unchanged | kept (moved from dock) |
| Hubble | Integrations; unchanged | kept |
| Linter visuals | Cluster › Linter section renders the same panel; still in Tools | kept (moved from dock) |
| Context tabs | untouched | kept |
| Certificates panel | Integrations › Certificates and Tools; panel unchanged | kept (moved from dock) |
| Small-window layout | phone/tablet tab strips unchanged (Topology tab kept); setup screen reflows to one column | kept |
| Status bar left side (#54) | untouched | kept |

## Group 3 interfaces

- `ui/src/layout/width-class.ts`: read only. `hasDock` (laptop and wider) still decides whether a dock exists.
- `ui/src/layout/defaults.ts`:
  - 043 owns `DOCK`, `DOCK_ALLOWED` and `filterDock`.
  - Group 3 owns `PROFILES`, slot definitions and `isValidDock`'s slot checks.
  - The merge rule: Group 3 keeps calling `filterDock` on whatever dock it resolves.
- `ui/src/store/index.ts`:
  - `assignPanes` keeps creating `dock-${tool}` panes, now only for allowed tools.
  - `navigate(section)` handles the new sections as ordinary `SectionKey`s rendered in list panes, so the cascade treats them like any other section.
  - `openInspector(ref, from, push)` is what the topology tab's "Follow" uses. If Group 3 changes the inspector's placement, "Follow" follows automatically.
- `AppShell.tsx`: the setup-screen branch wraps the whole slot area. Group 3 changes happen inside that branch's "else".
