# Live validation: 002 Pod Logs & Terminal

- **Date**: 2026-09-26
- **Cluster**: `kind-fathom-dev` (Kubernetes 1.32), namespace `fathom-dev`
- **How**: `ui/tests/live/qs-002-logs-terminal.spec.ts` through the devbridge (`--project=chromium-wide`,
  `FATHOM_LIVE_CONTEXT=kind-fathom-dev`). It creates its own fixture pods (a two-container busybox pod, a second
  busybox pod, a pause-image pod with no shell) and deletes them afterwards.

## Scenario 1: Logs tab

| Step | Result | Notes |
|---|---|---|
| 1.1-1.3 history within 5s, then live lines (SC-001) | PASS | asserted < 5000 ms from tab click to first lines |
| 1.4 container picker switches the stream | PASS (after fix) | run 4 failed: the stop/start pair raced (see bugs) |
| 1.5 scroll up stops follow, "Jump to latest" resumes it | PASS | |
| 1.6 leaving the tab sends `StopPodLogs` (FR-005) | PASS | |
| 1.7 crashed container shows "stream ended" | PASS | `crashloop` sandbox pod |

## Scenario 2: Terminal tab

| Step | Result | Notes |
|---|---|---|
| 2.1-2.3 attach and `ls /` output within 10s (SC-002) | PASS (after fix) | keystrokes had been reordered ("ehco") |
| 2.4 container picker shown before any `StartPodExec` (FR-008) | PASS | |
| 2.5 pty size matches the terminal and follows a resize (FR-010) | PASS (after fix) | checked with busybox `ttysize` (`stty size` fails with ENOENT on this kind node) |
| 2.6 closing ends the session server-side (FR-011, SC-003) | PASS (after fix) | a `sleep` started in the shell is gone after close |
| 2.7 a second pod's terminal ends the first session | PASS | |
| 2.8 a container with no shell shows a clear error, not a hang | PASS | pause image |

## Scenario 3: Promo (T031)

`FATHOM_LIVE_CONTEXT=kind-fathom-dev pnpm -C ui promo` gave 30 passed, 1 flaky and 4 failed. All
four failures were environmental: the Hubble scenes (no Hubble on this kind cluster) and the 036
daemon scenes (the daemon is not installed). The write guard recorded **zero writes**. The guard's
`WRITE_CMD` did not count `UninstallHelmRelease`, `StartPodExec`, `SendPodExecInput` or
`StartNodeShell`, so it was widened in both the promo and live helpers. The brochure PDF was not
regenerated, because brochure work is out of scope for this pass.

## Bugs found and fixed

1. Backend commands each ran on their own task and raced for the state lock, which reordered
   terminal keystrokes. Exec commands now run in one ordered lane.
2. `StopPodExec` left the shell and its children running, because the runtime does not kill
   exec'd processes on disconnect. Stop now sends Ctrl-C and Ctrl-D, then aborts.
3. The remote pty stayed at 80x24, because the first fit ran before the session existed.
   `TerminalView` now sends the size once the session is active.
4. The log stream's `StopPodLogs`/`StartPodLogs` pair on a container switch could swap, which
   killed the new stream. It now uses the same ordered lane.
