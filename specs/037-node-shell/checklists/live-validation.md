# Live validation: 037-node-shell (T030, Linux part)

Run 2026-09-26 on Linux (Pop!_OS 24.04, Ubuntu-based; kernel 7.1.5; Docker 29.8.1). Only the throwaway kind cluster `fathom-fed-5` (node `fathom-fed-5-control-plane`) was used, with the image `busybox:stable`. Node shell was **never** run against a production or shared context.

**Harness.** The real backend ran headless via `examples/live_driver.rs`: `StartNodeShell`, then `StartPodExec` for the terminal I/O, then `StopNodeShell`, exactly as `NodeShellPanel` sends them. Pod state was checked independently with `kubectl get pods -A -l fathom.io/node-shell=true`. A second context, `ns-limited` (a ServiceAccount bound to `view` in `default`), served as the limited context.

| Step | Result |
|------|--------|
| 1. Access and settings (admin), cancel | All checks `Allowed` (create/delete/get pods, create pods/exec, get nodes). Settings defaults: `busybox:stable`, ns `default`, deadline 14400. No pod exists before confirmation. **Pass.** |
| 2. Start and attach | `Creating` → `Pending` (with `ContainerCreating` detail) → `Running` at 2.1s → `Attached` at 2.8s. The pod spec has `nodeName` pinned, and `hostPID`, `hostNetwork` and `privileged` are all true. It has `activeDeadlineSeconds` and the `managed-by`/`node-shell`/`node` labels. In the shell, `hostname`, `ls /host/etc` and `chroot /host cat /etc/hostname` all returned `fathom-fed-5-control-plane` or the node's files. **Pass.** |
| 3. Cleanup paths | Each of these left 0 pods, checked with kubectl. **Pass.** <ul><li>Stop gave `Removed`.</li><li>`exit` gave StreamClosed (`exit`), then Stop gave `Removed`.</li><li>A second Node shell superseded the first: the old pod went `Terminating` then `Removed` (2.9s), and only the new pod remained.</li><li>A context switch gave `Terminating` then `Removed` (2.9s).</li><li>Quitting the app (`Shutdown`) removed the pod within 2s.</li></ul> |
| 4. Hard kill, orphans, deadline | After `kill -9` of the app, the pod was still `Running`. On relaunch, `ListNodeShellOrphans` listed it with its node, and `DeleteNodeShellOrphans` removed it, leaving 0 pods. With `deadline_seconds: 300` the cluster moved the pod to `Failed` by itself; a later supersede deleted it. **Pass.** The backend only polls pod phase while starting (by design, R7), so a detached session is not told about the deadline expiry. The spec requirement is that the cluster stops the pod without Fathom, which it did. |
| 5. Limited context | `create pods`, `delete pods`, `create pods/exec` and `get nodes` were `Denied`; `get pods` was `Allowed`. The UI uses this list to show the denied reasons instead of the dialog. **Pass.** |
| 6. PodSecurity `restricted` namespace | The start `Failed` with the verbatim admission error (`violates PodSecurity "restricted:latest": host namespaces (hostNetwork=true, hostPID=true, hostIPC=true), privileged ...`), and no pod was created. **Pass.** |
| 7. Nonexistent image | `Pending` repeatedly showed `ErrImagePull` / `ImagePullBackOff: Back-off pulling image ...` with the registry's verbatim reason. Stop gave `Removed`, leaving 0 pods. **Pass.** |
| 8. Layouts | Not exercised live. The fake-backend Playwright spec checks no horizontal scroll at all widths. |
| 9. Tier 1 OSes | **Linux only**, run on Pop!_OS 24.04 rather than Ubuntu 26.04. Windows 11 and macOS 26 were not run. |
| 10. Gates | See the T030 note in tasks.md. |

Not covered live:
- closing the terminal tab in the real window: there is no WebDriver on this host, and the backend path is the same `StopNodeShell` as the Stop step;
- the Tauri Ctrl+C grace period.
