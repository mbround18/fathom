# Research: Setup and navigation

## Setup screen placement

- **Decision**: A branch in `AppShell` renders `SetupScreen` instead of the navigator, the slot grid or phone stack, and the dock. It is driven by a pure `setupStateOf()` built on `firstRunOf`.
- **Rationale**: First run is an app-wide state. Rendering it once removes the repeated per-pane message (feedback #4) and keeps the top bar and status bar live. The status bar's reload outcome is feedback #1, and its left side is #54.
- **Alternatives**:
  - A modal dialog over the shell: rejected because it blocks the top bar and status bar.
  - A special pane in slot 0: rejected because it depends on the slot model that Group 3 is replacing.

## Which states use it

- **Decision**: The setup screen covers only the `kubeconfig` and `pick` first-run kinds, plus the connecting and failed states *reached from setup*. The `connection` kind for an already-selected context stays per-pane and gains "Choose another context".
- **Rationale**: A connection failure after the app was in use should not throw the operator out of their layout. The context tabs already show per-context status.

## Remembering the picked context

- **Decision**: Store the context in `UiConfig.last_selection` (`{context, namespace?, section: "overview"}`) through `SaveUiConfig`. At startup, use it only if the kubeconfig has no valid `current-context` and the remembered name still exists.
- **Rationale**: The field already exists in the protocol and is unused, so no protocol change is needed. Principle VI and "never blackbox" forbid writing the operator's kubeconfig.
- **Alternatives**:
  - Writing `current-context` to the kubeconfig: rejected, since it changes behaviour for kubectl and other tools.
  - A new config field: rejected as unnecessary.

## Sign-in step

- **Decision**: An optional step that composes the existing `IssuerSettings`, `SignInPanel`, `ContextAuth` and the exec-plugin status (035). The same composition is the Settings › Sign-in page, and the header IdentityChip dialog keeps working.
- **Rationale**: Feedback #5 said sign-in belongs in setup and/or settings. Composing the existing pieces keeps one implementation.

## Settings structure

- **Decision**:
  - The Settings nav group holds `integrations/cluster-settings` (label "Cluster settings", key kept), new `settings/sign-in`, `team/teams`, `team/profiles`, new `settings/app`, new `settings/local-clusters` (desktop only) and `audit-log`.
  - The header `SettingsMenu` is placed immediately before `IdentityChip`, and both move to the right action cluster.
- **Rationale**: Keeping the existing keys preserves saved routes and slot pins (FR-013). Feedback #9/#15 asks for a header entry "left of sign-in, together with team settings".
- **Alternatives**: Renaming the keys to `settings/*` was rejected: it needs migration of persisted slot sections for no user-visible gain.

## Daemon naming

- **Decision**:
  - Keep "daemon". The chip shows "Daemon: <state>", and the switch reads "Use Fathom's in-cluster daemon" with an on/off description.
  - "While you were away" becomes "Cluster history".
- **Rationale**: "Daemon" matches spec 036, `k8s/overlays/daemon` and the docs. The confusion in feedback #12 came from the missing label and explanation, not from the word. "Cluster history" says what the panel is.
- **Alternatives considered**: "In-cluster helper" and "Fathom agent". Both were rejected: "agent" collides with AI agents in App settings, and "helper" is vague.

## Dock policy

- **Decision**:
  - `DOCK_ALLOWED = logs, events, daemon`. The default is `logs, events`, and `daemon` is appended while it is available.
  - `filterDock` is applied in `resolveProfile`.
  - The enum is unchanged.
- **Rationale**:
  - The dock is a persistent area under the panes. It is useful for things that keep flowing while you browse (log tail, events, history feed). The rest are destinations.
  - Principle IV keeps a persistent tool area from laptop width up.
  - Principle VI rules out destructive migration.
- **Alternatives**:
  - Removing the dock: breaks Principle IV and loses the log tail that operators use most.
  - Letting users re-add any tool: this reintroduces the clutter, and Tools already opens any tool in a sheet.

## Per-resource topology

- **Decision**: An inspector tab driven by `RefreshRelatedResources`, with the relation mapped to direction as follows:
  - Upstream: owned-by, member-of, scaledBy, mounted-by, reconciled-by.
  - Downstream: owns, selects, scales, scheduled-on, and anything else.

  "Follow" is `openInspector(ref, from, true)` with the tab kept.
- **Rationale**: The resolver already covers the kinds in feedback #17's "follow the chain", and the inspector already has back history. One hop per view, re-rooted by Follow, is enough to walk from Service to Pod to Node.
- **Alternatives**: Embedding a Cytoscape graph per resource was rejected: it is heavier, and in-place expand/collapse is #23's scope.

## Navigator model

- **Decision**:
  - A single data model (`NAV_MODEL`) holds groups, entries, descriptions, the `advanced` flag and visibility rules.
  - "More" state is stored as `"<Group>/More"` in the persisted `expanded_groups`.
  - `LEGACY_GROUP_LABELS` maps the old group names.
- **Rationale**: One source for the tree, the palette and the coverage test. Reusing `expanded_groups` avoids a protocol field. The strings are opaque to the backend.
- **Alternatives**: Separate description tables per consumer were rejected because they would drift.

## Velero visibility

- **Decision**: `integrationVisible('velero', installed)` returns true only when `installed === true`. Every consumer (nav, Tools, tablet tabs, palette) uses it.
- **Rationale**: The user asked for this explicitly, and "unknown = hidden" avoids flicker.

## Existing tests touched

| Spec | Change |
|---|---|
| `first-run-states.spec.ts` | kubeconfig/pick cases assert the setup screen; connection cases unchanged, plus "Choose another context" |
| `shell.spec.ts`, `responsive-shell.spec.ts`, `a11y-shell.spec.ts` | header order (Settings before IdentityChip), dock tabs Logs/Events |
| `nav-walk.spec.ts`, `nav-integrations.spec.ts` | new group names; Velero hidden; Settings group |
| `us1-daemon.spec.ts` | "Cluster history" label, switch description |
| `us1-topology-overview`, `us2-topology-zoom`, `us3-topology-inspector`, `us4-topology-live-update` | open Topology via Cluster › Topology instead of the dock tab |
| `velero-pane.spec.ts` | mark Velero installed before asserting the pane |
| `us2-teams-cluster-settings.spec.ts`, `us6-settings.spec.ts`, `observability-settings.spec.ts`, `mcp-settings-persistence.spec.ts` | settings reached from the Settings menu; items moved to App settings |
| `us1-oidc-identity.spec.ts`, `batch-i-ai-identity-portforward.spec.ts` | IdentityChip position; sign-in page |
| `navModel.test.ts`, `navCoverage.test.ts`, `layout-defaults.test.ts`, `paletteIndex.test.ts`, `first-run.test.tsx` | rewritten for the new model, keeping every previous assertion's intent |
| `tests/promo/compose.spec.ts` | 'While you were away' → 'Cluster history'; new captures last |
