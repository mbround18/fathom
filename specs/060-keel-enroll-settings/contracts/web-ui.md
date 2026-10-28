# Contract: Keel web UI (spec 060)

## Navigation

- **System Settings**: a nav entry pinned to the bottom of the left nav, shown when
  `me.instance_role` is owner or admin. Route `/settings/:tab`.
- **Tabs**:

  | Tab | Visible to |
  |---|---|
  | `enrollment` | admins and owners |
  | `tokens` | admins and owners |
  | `identity` | admins and owners; read-only role mapping |
  | `system` | owners only |

  Unknown or forbidden tabs redirect to `enrollment`.
- **Loading**: every page and the Markdown renderer load with `React.lazy`.

## Hosts: Enroll dialog

- **Button**: "Enroll" in the Hosts page header. It is shown when the caller can operate on at
  least one workspace (R6).
- **Step 1, the form**:
  - workspace, defaulting to the current one;
  - inventory name, validated as today;
  - profiles, as multi-select chips.

  "Continue" previews; confirming does step-up as today.
- **Step 2, the result**:
  - the token in a `CopyBlock` (masked, with a reveal);
  - the snippet in a `CopyBlock` (copy only);
  - the instructions rendered as Markdown below;
  - the expiry as a countdown.

  Closing the dialog warns that the token won't be shown again.
- **Removed**: the old inline token box and its wrong `-e keel_enroll_token=` snippet.

## Settings: Enrollment tab

- **Template editor**: monospace, with the placeholder reference beside it. Inline validation uses
  the same codes as the server (`missing_token`, `unknown_placeholder`, `too_long`). The live
  preview uses a dummy token.
- **Instructions editor**: Write and Preview tabs.
- **Actions**: Save (step-up), "Restore default", and a conflict banner with Reload.

## Settings: Tokens tab

- **Enrolment tokens**: a `DataTable` with a state filter. Row actions are Revoke and Roll, both
  step-up. Roll opens step 2 of the Enroll dialog with the new token.
- **API tokens**: a `DataTable` showing holder, workspace, actions, state, last used and expiry.
  Revoke uses step-up when the token isn't yours.

## Users page

- **Identity-managed rows**: rows with `role_source = idp` show an "Identity provider" tag, and no
  role control.

## Shared primitives (`src/ui/`)

`Dialog`, `CopyBlock`, `EmptyState`, `LoadState`, `Tabs`, `DataTable` and `PageHeader`. Every
section uses them after the polish pass (FR-031, R10).

## Tests

- **Vitest**:
  - `template.ts` against `contracts/template-vectors.json`;
  - the Markdown renderer (raw HTML shown as text, images become links, links open in a new tab);
  - the tab visibility rules.
- **Playwright**, one spec per feature:
  - `enroll.spec.ts`;
  - `settings-enrollment.spec.ts`;
  - `settings-tokens.spec.ts`;
  - `settings-system.spec.ts`;
  - `polish-screens.spec.ts` (360 and 1280 px, both themes).
