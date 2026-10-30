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
| Workspace picker (no workspace) | Create a workspace | Hosts, bindings and runs all live in a workspace. | 1. Create a workspace. 2. Grant a team access. 3. Enroll a host into it. | "Create a workspace" → `/teams` (owner or admin, `canSeeSettings`) | `getting-started#create-a-workspace` |
| Hosts | Enroll a host | Keel can only run on hosts that have enrolled. | 1. Enroll a host to get its command. 2. Run the command on the host. 3. Run the base profile. | "Enroll a host" (`canOperate(me, ws)`) | `hosts#enroll-a-host` |
| Receivers | Create a receiver | A receiver is how your Git host tells Keel about a push. | 1. Create a receiver. 2. Add its URL and secret as a webhook at your Git host. 3. Push to see the first delivery. | "Create a receiver" (the page's existing guard) | `receivers#create-a-receiver` |
| Bindings | Bind a repository | A binding says which branch Keel runs, and where. | 1. Add a credential for the repository. 2. Bind a branch to this workspace. 3. Push to the branch. | "Bind a repository" (existing grant check) | `bindings#bind-a-repository` |
| Runs | No runs yet | A run applies a bound branch to its hosts. | 1. Bind a repository. 2. Push to the bound branch. 3. Approve the review if asked. | "Go to bindings" → `/bindings` (same as Bindings) | `getting-started#your-first-run` |
| Reviews (queue) | Nothing to review | Changes wait here when the early check can't trust them. | 1. Add trusted signers to skip reviews for their commits. 2. Push a change. | "Trusted signers" → `#trusted-signers` on the same page (whoever may edit signers) | `reviews#trusted-signers` |
| Inventory | No inventory yet | Inventory lists the hosts Keel manages and the repository's own. | 1. Enroll a host. 2. Run the base profile. 3. Or import the repository's inventory. | "Go to hosts" → `/hosts` (`canOperate`) | `inventory` |
| Teams: workspaces | Create a workspace | (same as the picker) | (same as the picker) | "Create a workspace" (owner or admin) | `getting-started#create-a-workspace` |
| Teams: teams | Create a team | Teams carry workspace grants to their members. | 1. Create a team. 2. Add members. 3. Grant it a workspace. | "Create a team" (existing guard) | `getting-started#give-people-access` |

The draft copy above is final only after `/impeccable clarify`. The rules hold regardless:

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
- `enrolment-token`, `fingerprint`, `preview-credential`;
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
