# Contract: web UI (spec 068)

Everything below is hidden while `KEEL_CONVERGE` is off (the server reports flags in
`SystemInfo`). Components reuse the existing shadcn primitives and the Keel H1/H2/H3 scale; no
new tracked caps. Run the impeccable skill over every screen here and its surroundings before
the story is done.

## Hosts (`features/hosts/HostsPage.tsx`)

- **Converge column** (priority after State; hidden < 640 px, folded into the row's second line):
  `converging 2/3 · keel-node`, `converged`, `failed at debian`, or empty.
- **Suggestion dot** next to the Profiles cell when a host has open suggestions; tooltip
  "2 suggested from inventory groups".
- **Bulk action** "Converge…" on a selection (Operate on all their profiles; otherwise disabled
  with the reason).

## Host page

- **Profiles card**: assigned profiles as chips; ceiling shown under it ("this host accepts:
  …"). Assigned-but-outside-ceiling chips are marked with a warning icon and text, never colour
  alone.
  - **Edit profiles** (admin): add/remove → Preview shows the diff, the new `profile:`
    principals, the plan after the change, and any profile the runner won't accept. Confirm asks
    for step-up.
  - **Suggestions** list: `+ k3s-agent (from k3s_agents)` with Accept (opens the same preview,
    pre-filled) and Dismiss.
- **Converge** button → Converge preview for this host. A **Check** toggle on the preview.
- **Last converge** summary: steps with state icons, link to the converge page.

## Converge preview (dialog, then full page on narrow screens)

- Header: binding, commit (short sha + subject), inventory revision, `check` badge when on.
- Matrix: rows = hosts, columns = step positions; each cell `job` with `via` on hover/focus
  ("from keel-node → debian"). < 640 px: one card per host, steps as an ordered list.
- Params and **credential names** per step (never values).
- Policy: "Needs 2 approvals, four eyes, expires in 4 h" (combined, with "strictest of N steps").
- Signed mode: a "Review" section listing each step's assertions, grouped by child run.
- Refusals per host (outside ceiling, runner too old) with the host excluded from Confirm.
- `replaces` / `blocked_by`: "Replaces unapproved converge from 10:02 (link)" or "Host X has a
  converge running (link); it will be left out".
- Confirm: "Converge 3 hosts (7 steps)".

## Converge page (`features/converge/ConvergePage.tsx`, new; route `/runs/:id` when kind is converge)

- The same matrix, live. Cell states: blocked (hollow), queued, running (spinner), succeeded,
  failed, lost, skipped (struck with "dependency failed"), covered, cancelled. Each icon has a
  text label for screen readers and in the cell at ≥ 1024 px.
- Click a cell → the child run's log for that host (existing run log view, filtered to host).
- Actions: Approve/Deny (existing approval UI on the parent), Cancel, **Retry failed** (preview
  shows only the steps not succeeded; option "Re-plan at latest commit").
- `retry_of` link at top when present.

## Runs list (`features/runs/`)

- Converge runs appear once (the parent); child runs are hidden from the list by default (filter
  "show steps").
- `check` badge on check runs everywhere runs are listed (FR-011).
- Host rows `covered` show "covered by converge (link)".
- `waiting_checks` state shows the contexts with their states and forge links.

## Start dialog (job run)

- Params section: `oci_digest` (existing) and `choice` (a select; options resolved at the pinned
  commit; empty `from:` shows "no entries under composes/").
- **Check** toggle.

## Enrollment (Settings → Enrollment, Add host)

- Checkbox "Converge after enrollment" (default on). Help text names the profiles and the plan
  preview ("will run: debian, keel-node").

## Binding page

- **Required checks** (admin): list of contexts, add/remove via preview with step-up; timeout
  field. When the forge can't read statuses: disabled with the reason.
- **Runner compatibility** banner when `runners_refusing_manifest` is non-empty: "3 runners would
  refuse this manifest (older than 068). Upgrade them with the keel-node job." with the host list.

## Playwright (`tests/e2e/s068/`, project `s068`, fake backend `tests/support/fake-converge.ts`)

`converge-preview.spec.ts`, `converge-page.spec.ts`, `host-profiles.spec.ts`,
`suggestions.spec.ts`, `check-mode.spec.ts`, `covered.spec.ts`, `required-checks.spec.ts`,
`choice-param.spec.ts`, `enroll-auto-converge.spec.ts`. Each at 360 px and 1280 px, keyboard
only for the confirm paths.
