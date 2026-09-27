# UI shell contract: Setup and navigation

No protocol changes. This contract fixes the observable UI surface that E2E tests assert.

## Commands emitted (all existing)

| Trigger | Command |
|---|---|
| Pick a context in setup | `SelectContext { context }`, then `SaveUiConfig` with `lastSelection.context` |
| Reload kubeconfig in setup | existing reload command (feedback #1) |
| Sign-in step | existing OIDC issuer / sign-in / context-auth commands (033) |
| Daemon switch | existing daemon preference command (036), `auto` / `off` |
| Inspector Topology tab | `RefreshRelatedResources { kind, namespace, name }` |
| Dock/layout change by the user | `SaveUiConfig` with the filtered dock |

Nothing in the setup screen, Settings pages or nav emits a mutating cluster command.

## Test ids and accessible names

| Element | Contract |
|---|---|
| Setup screen | `data-testid="setup-screen"`; the only child of the main area when shown |
| Context row | `role="option"` in a `role="listbox"` named "Contexts"; text is the context name verbatim |
| Filter | `role="searchbox"` named "Filter contexts" |
| Sign-in step | heading "Sign in (optional)", button "Skip" |
| Header Settings | `role="button"` named "Settings", the DOM sibling immediately before the identity chip |
| Settings menu items | Cluster settings, Sign-in, Team, Connection profiles, App settings, Local clusters (desktop), Audit log |
| Daemon chip | visible text starts with "Daemon" at laptop width and wider; accessible name "Daemon: <state>" at every width |
| Daemon switch | accessible name "Use Fathom's in-cluster daemon"; `aria-describedby` points to the on/off text |
| Dock tabs | exactly `Logs`, `Events`, and `Cluster history` when the daemon is available |
| Nav group | `role="treeitem"` with label from FR-020 order |
| Nav entry | `aria-describedby` points to its description; tooltip shows the same text |
| More toggle | `role="button"` named "More <group>", `aria-expanded` |
| Inspector Topology tab | tab named "Topology"; sections "Upstream" and "Downstream"; each neighbour has a "Follow" button |
| Role picker | option "Create a new role…" at the end of the role picker |

## Persistence

- `layouts.<class>.dock.tabs` is read through `filterDock`. It is written only when the user changes the layout.
- `navigation.expanded_groups` may contain the new group labels and `"<Group>/More"` entries. Old labels are mapped on read and are not rewritten until the next change.
- `lastSelection` holds the remembered context.
