# UI contract: Keel run approval

## Components

```ts
// features/hosts/workspaces.ts (next to canOperate, canAdminister)
canApprove(me: Me | null, ws: string): boolean   // approver, ws_admin, owner, admin; mirrors authz.rs

// features/runs/approval.ts (pure; unit tested)
approvalView(i: { run: RunInfo; me: Me | null }):
  | { kind: 'act' }                 // Approve + Deny
  | { kind: 'requester' }           // four-eyes starter: Deny only + "Someone else must approve this run."
  | { kind: 'answered' }            // already approved: no buttons, "You approved this run."
  | { kind: 'restricted' }          // "An approver or workspace admin can approve this."
  | { kind: 'closed' }              // queued / denied / expired: history only

// features/runs/ApprovalPanel.tsx
ApprovalPanel(props: { run: RunInfo })

// features/runs/approvals-store.ts
useApprovals(): { total: number; byWorkspace: Record<string, number>; refresh(): void }
```

## Behaviour

- **ApprovalPanel**, on `RunPage`, above the hosts. It shows:
  - the policy, as "Needs N approval(s)", with "four-eyes" when it applies;
  - "{k} of {N} approvals";
  - each approval: the name, role and relative time;
  - "Window closes {relative} ({absolute})";
  - for a denied run, the denier and the reason.
- **Approve** is the primary button. It opens the existing preview dialog. The preview text names
  the job, the inventory, the hosts and the commit, and what the approval does: "queues the run",
  or "adds approval {k+1} of {N}".
- **Deny** uses the destructive variant. It opens the preview dialog with an optional "Reason"
  textarea, `maxLength` 500, with a counter.
- Step-up is handled by the existing confirm flow. If the person cancels it, nothing changes.
- A refusal from the server shows through the existing `OpError` path, in the dialog, using the
  server's message.
- **RunsPage**:
  - a segmented filter, "All" or "Waiting for approval", synced to `?state=pending_approval`;
  - in the filtered view, rows are oldest first;
  - a held row's state cell reads "Waiting · 1 of 2".
  - When the filtered list is empty, the page shows the 063 `NoMatch` ("No runs waiting for
    approval.").
- **Nav**: the Runs link shows `useApprovals().total` as a count chip when it is above 0 and the
  person can approve somewhere.
  - The chip is neutral (`bg-muted text-foreground`), never the accent.
  - Its `aria-label` is "{n} runs waiting for your approval".

## Test hooks

| Attribute | On | Values |
|---|---|---|
| `data-approval-panel` | ApprovalPanel root | the `approvalView` kind |
| `data-approval-count` | "k of N" text | `k/N` |
| `data-approve` / `data-deny` | the buttons | present |
| `data-runs-filter` | filter root | `all`, `pending_approval` |
| `data-nav-badge="runs"` | nav chip | the text is the count |
