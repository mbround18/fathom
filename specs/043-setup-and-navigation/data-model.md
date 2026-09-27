# Data model: Setup and navigation

All entities are UI-side. The only persisted data are existing fields: `UiConfig.last_selection`, `UiConfig.layouts[*].dock` and `navigation.expanded_groups`.

## SetupState (derived, not persisted)

```text
SetupState =
  | { kind: 'kubeconfig', issue: KubeconfigIssue, paths: string[] }   // missing / empty / malformed / unreadable
  | { kind: 'pick', contexts: string[], paths: string[] }
  | { kind: 'connecting', context: string }
  | { kind: 'failed', context: string, problem: ConnectionProblem }
  | { kind: 'sign-in', context: string }
  | undefined                                                        // normal shell
```

- The input is `setupStateOf({ kubeconfig, activeContext, connection, hosted, pickedInSetup, signInStep })`.
- `hosted === true` always gives `undefined`.
- `connecting` and `failed` apply only when `pickedInSetup` is set. Otherwise a connection problem is a per-pane `FirstRun{kind:'connection'}`, as today.
- Transitions:
  - `pick` → `connecting` (SelectContext) → shell. `sign-in` comes before the shell if it is offered and not skipped.
  - `failed` → `connecting` (Retry), `sign-in`, or `pick`.

## RememberedContext (persisted, existing field)

`UiConfig.last_selection = { context, namespace?, section: 'overview' }`

- Written when a context is picked in setup.
- Read at startup. It is used only if the kubeconfig has no valid `current-context` and `context` is in the kubeconfig's list.

## NavModel (static data)

```text
NavGroup  = { label: string, entries: NavEntry[], defaultExpanded: boolean }
NavEntry  = { section: SectionKey, label: string, description: string,
              advanced?: true, integration?: IntegrationKey,
              visible?: (ctx: { hosted, desktop, installed(key) }) => boolean }
LEGACY_GROUP_LABELS = { Overview: ['Cluster'], Network: ['Networking'],
                        Config: ['Configuration','Scaling'], Team: ['Settings'] }
```

- The group order and entries follow FR-020.
- Rules:
  - `description` is at most 90 characters, plain language, and does not repeat the label.
  - `advanced` entries render under "More", whose persisted key is `"<label>/More"` in `expanded_groups`.
  - Velero's `visible` is `installed('velero') === true`.
  - `settings/local-clusters` has `visible` = desktop and not hosted.
- New section keys: `topology`, `linter`, `cluster-history`, `settings/sign-in`, `settings/app`, `settings/local-clusters`.

## DockPolicy (static data plus a pure function)

```text
DOCK_ALLOWED: ToolKind[] = ['logs', 'events', 'daemon']
DOCK_DEFAULT: DockConfig = { open: true, heightPct: <unchanged>, tabs: ['logs', 'events'] }
filterDock(saved: DockConfig): DockConfig      // drops tabs not in DOCK_ALLOWED, keeps order/open/heightPct
dockTabs(dock, daemonAvailable): ToolKind[]    // removes 'daemon' when unavailable, appends it when available and absent
```

## ResourceTopology (derived)

```text
ResourceTopology = { root: RelatedResourceRef-like, upstream: Neighbour[], downstream: Neighbour[] }
Neighbour        = RelatedResourceRef & { relationLabel: string }
UPSTREAM = { 'owned-by', 'member-of', 'scaledBy', 'mounted-by', 'reconciled-by' }
```

- `RESOLVER_KINDS`: Node, Service, PersistentVolumeClaim, Pod, Deployment, StatefulSet, DaemonSet, ReplicaSet, ScaledObject, ScaledJob, Application, HelmRelease. The Topology tab is shown only for these kinds.
- Events whose root does not match the inspector's current ref are discarded (Principle V).
