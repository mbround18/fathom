# Quickstart: Validating Pod Logs & Terminal

## Prerequisites

- A running Kubernetes context reachable via your kubeconfig, with at least one running,
  multi-container-capable pod (any workload with a shell-having base image, e.g. anything not
  `scratch`/distroless).
- Backend built with this feature's changes; frontend built with `xterm` + `@xterm/addon-fit`
  installed (`pnpm -C ui install` after `package.json` is updated).

## Run the app

```bash
FATHOM_LIVE_CONTEXT=<your-context> cargo run
# or, for the fake-backend frontend dev loop while iterating on UI only:
pnpm -C ui dev:fake
```

## Scenario 1 — Logs tab (User Story 1, P1)

1. Select a running pod in the resource list.
2. Open the Inspector's **Logs** tab.
3. **Expected**: recent log history appears within a few seconds (SC-001), followed by live lines
   as the container produces them.
4. If the pod has >1 container: confirm a container picker is shown and switching containers
   restarts the stream with the new container's output.
5. Scroll up: confirm auto-scroll stops (FR-004) and a "jump to latest" control appears; use it,
   confirm auto-scroll resumes.
6. Switch to the Details tab or close the Inspector: confirm (via backend logs/tracing, or a debug
   build assertion) the log stream task is aborted — no orphaned stream (FR-005, SC-003).
7. Select a pod whose container has stopped/crashed: confirm a distinct "stream ended" indicator
   is shown (FR-006), not a silent blank pane.

## Scenario 2 — Terminal tab (User Story 2, P2)

1. Select a running pod (with a shell-capable container).
2. Open the Inspector's **Terminal** tab.
3. **Expected**: shell attaches within ~10s (SC-002); type `ls` and `Enter`, see real directory
   listing output from inside the container.
4. If the pod has >1 container: confirm the container picker appears *before* the session starts
   (FR-008).
5. Resize the Inspector pane or window: run `top` inside the terminal, confirm it redraws to fill
   the new size correctly (FR-010) rather than corrupting/wrapping wrong.
6. Close the tab / switch away / close the Inspector: confirm the exec session is aborted
   server-side — no orphaned session (FR-011, SC-003).
7. Open Terminal against a second pod while the first session is still open: confirm the first
   session is cleanly ended before the second attaches (FR-012, one active session app-wide).
8. Attempt Terminal against a pod/container with no shell binary, or (if testable) without
   `pods/exec` RBAC: confirm a clear, specific error is shown (FR-013), not a hang or blank pane.

## Scenario 3 — Promo brochure (User Story 3, P3)

1. `FATHOM_LIVE_CONTEXT=<demo-context> pnpm -C ui promo` — confirm the new `wide-logs` scene opens
   a pod's Logs tab against the real demo cluster, waits for real streamed lines, and captures a
   screenshot, with `expectNoWrites`/the existing `WRITE_CMD` guard reporting zero writes
   throughout the whole run (FR-014).
2. Confirm the automated run never opens/drives the Terminal tab — grep the run's command log (or
   `capture.spec.ts` itself) for any of `StartPodExec`/`SendPodExecInput`/`ResizePodExec` and
   confirm zero occurrences (FR-015).
3. Confirm the regenerated `promo-out/fathom-promo.pdf` includes a Logs page (live shot, branded
   header/footer/watermark/page-numbering matching existing pages) and a Terminal page (using the
   staged screenshot asset from `ui/tests/promo/assets/`), per FR-016 and SC-004.

## Success criteria mapping

| Scenario step | Success Criterion |
|---|---|
| 1.2–1.3 | SC-001 (<5s to live logs) |
| 1.6, 2.6 | SC-003 (zero orphaned streams on navigate-away) |
| 2.2–2.3 | SC-002 (<10s to typed-command output) |
| 3.1–3.3 | SC-004 (real logs screenshot + terminal showcase, zero cluster-affecting commands during unattended capture) |
