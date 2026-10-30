# Data model: Keel guided UI

All of this is client-side and static: types and constant tables in `apps/keel/web/src`. Nothing
is stored, and nothing goes over the wire.

## Section and NavGroup (`app/nav.ts`)

| Field | Type | Rule |
|---|---|---|
| `Section.path` | `string` | Unchanged. |
| `Section.label` | `string` | Unchanged; sentence case. |
| `Section.icon` | `Icon` | Unchanged. |
| `Section.visible` | `(me: Me) => boolean` (optional) | Absent means visible. |
| `NavGroup.id` | `'pipelines' \| 'infrastructure' \| 'administration'` | Used in `data-nav-group`. |
| `NavGroup.label` | `string` | `Pipelines`, `Infrastructure`, `Administration` (FR-001, FR-002). |
| `NavGroup.sections` | `readonly Section[]` | In this order (FR-001). |

The groups, in order:

1. **Pipelines**: Runs, Reviews, Bindings, Receivers.
2. **Infrastructure**: Inventory, Hosts.
3. **Administration**: Teams, Users, Audit, Migration.

The rules:

- `visibleGroups(me)` keeps each group's sections whose `visible` is absent or returns true, then
  drops groups with none (FR-004).
- `SECTIONS = NAV_GROUPS.flatMap(g => g.sections)`.
- `SETTINGS` stays outside the groups (FR-005).

## Guided empty state (`ui/EmptyState.tsx`, `ui/empty-variant.ts`)

`EmptyState` props. The new ones are marked †:

| Prop | Type | Rule |
|---|---|---|
| `title` | `string` | Short, sentence case. |
| `reason` † | `ReactNode` | One line on why it matters (FR-011). |
| `steps` † | `ReactNode[]` | Two or three items; rendered as `<ol>` (FR-011). |
| `action` | `ReactNode` | The page's one primary button; omitted when the person can't act. |
| `whoCan` † | `string` | Shown only without `action`: "{whoCan} can {verb}." (FR-012). |
| `verb` † | `string` | For example "create receivers". |
| `guide` † | `GuideRef` | Rendered as a guide link (FR-011, FR-030). |
| `icon`, `children` | as today | Existing callers are unchanged; no illustrations (FR-017). |

`emptyVariant({ filtered, canAct })` decides the variant:

- it returns `'no-match'` when `filtered`;
- otherwise `'guided'` when `canAct`;
- otherwise `'restricted'`.

### The pages (FR-010)

The "can act" column uses the checks pages already make. Implementation confirms each against the
page's current button guard.

| Page | Title | Reason (one line) | Steps | Action (who can act) | Guide |
|---|---|---|---|---|---|
| Workspace picker (no workspace) | Create a workspace (owner or admin); No workspace yet (everyone else) | Hosts, bindings and runs all live in a workspace. | 1. Create a workspace. 2. Grant a team access to it. 3. Enroll a host into it. (Owner or admin only.) | "Create a workspace" → `/teams` (owner or admin, `canSeeSettings`); else "An owner or admin can grant you a workspace from Teams." | `getting-started#create-a-workspace` |
| Hosts | Enroll a host (No hosts yet) | Keel only runs on hosts that have enrolled. | 1. Enroll a host to get its one-time command. 2. Run the command on the host. 3. Run the base profile on it. | "Enroll a host" (`canOperate(me, ws)`) | `hosts#enroll-a-host` |
| Receivers | Create a receiver (No receivers yet) | A receiver is how your Git host tells Keel about a push. | 1. Create a receiver. 2. Add its URL and secret as a webhook at your Git host. 3. Push to see the first delivery. | "Create a receiver", focuses the new receiver's name (`canOperate`) | `receivers#create-a-receiver` |
| Bindings | Bind a repository (No repository is bound yet) | A binding says which branch Keel runs, and where. | 1. Add a credential if the repository is private. 2. Bind a branch to this workspace. 3. Push to the branch. | "Bind a repository", opens the New binding form (`canOperate`) | `bindings#bind-a-repository` |
| Bindings: credentials | Add a credential (No credentials yet) | A private repository needs a token or SSH key before Keel can fetch it. | 1. Add a token or SSH key. 2. Grant it to the binding that needs it. | "Add a credential" (outline: Bindings holds the page's primary action), focuses the name (`canOperate`) | `bindings#credentials` |
| Runs | No runs yet | A run applies a bound branch to its hosts. | 1. Bind a repository. 2. Push to the bound branch. 3. Approve the review if Keel asks. | "Go to bindings" → `/bindings` (`canOperate`) | `getting-started#your-first-run` |
| Reviews (queue) | Nothing to review | A change waits here when no trusted signer signed its commit. | 1. Add trusted signers, whose signed commits skip review. 2. Push a change to a bound branch. | "Go to trusted signers", scrolls to the page's signers section (`canOperate`) | `reviews#trusted-signers` |
| Inventory | No inventory yet | An inventory lists the hosts a run targets, and their vars. | 1. Enroll the hosts it will list. 2. Create a managed inventory here, or sync one from a bound repository. | "Go to hosts" → `/hosts` (`canOperate`) | `inventory` |
| Teams: workspaces | Create a workspace (No workspaces yet) | (same as the picker) | (same as the picker) | "Create a workspace" (owner or admin) | `getting-started#create-a-workspace` |
| Teams: teams | Create a team (No teams yet) | Teams carry workspace grants to their members. | 1. Create a team. 2. Add members. 3. Grant it a workspace. | "Create a team" (owner or admin) | `getting-started#give-people-access` |

A title in brackets replaces an imperative one for someone who can't act. Restricted variants read "An operator or workspace admin can {verb}." on workspace pages and "An
owner or admin can {verb}." on Teams. The server requires Administer for receivers, credentials,
signers, inventory and bindings, which is narrower than `canOperate`; the guided state follows the
page's existing client guard, so an operator can be offered an action the server then refuses.

The copy above is final after `/impeccable clarify` and `/impeccable onboard` (T028). The rules hold:

- one line of reason;
- at most three steps;
- one primary action;
- sentence case.

Secondary lists keep their one-liners (FR-016): deliveries, revisions, and hosts on a run.

## Glossary entry (`help/glossary.ts`)

| Field | Type | Rule |
|---|---|---|
| key | `TermId` (literal union) | kebab-case, for example `trusted-signers`. |
| `term` | `string` | As shown, in sentence case. |
| `text` | `string` | One or two sentences, at most 240 characters (FR-020). |
| `guide` | `GuideRef` (optional) | Must resolve (SC-006). |

The required terms (FR-020):

- `binding`, `receiver`, `trusted-signers`, `early-check`, `review`, `base-profile`;
- `enrolment-token`, `fingerprint`, `credential`;
- `inventory-managed`, `inventory-repository`;
- `workspace`, `team`, `grant`, `profile`.

The draft text is in [contracts/glossary.md](contracts/glossary.md).

## Guide (`help/guides.ts`)

| Field | Type | Rule |
|---|---|---|
| `GuideSlug` | literal union | One per `docs/guides/keel/*.md` file, without `.md`. `README` maps to the `/help` index. |
| `GuideRef.guide` | `GuideSlug` | — |
| `GuideRef.anchor` | `string` (optional) | Must equal `slug(heading)` of an `##` or `###` heading in that guide. |
| `guideHref(ref)` | `string` | `/help/<slug>` plus `#<anchor>` when given. |

The new guides (FR-030):

- `getting-started`, `hosts`, `receivers`, `bindings`, `reviews`, `inventory`.

The existing guides:

- `enrollment-template`, `oidc-roles`, `system-workspace`, `updates`.
