# Research: Node Shell

## R1. Pod shape (kubectl debug node parity)
- Decision: create a bare Pod with `nodeName`, `hostPID/hostNetwork/hostIPC`, privileged container, hostPath `/` at `/host`, `restartPolicy: Never`, tolerate all taints.
- Rationale: matches `kubectl debug node/<n> --profile=sysadmin`. Setting `nodeName` bypasses the scheduler, so cordon does not block (kubectl behaves the same).
- Alternatives: DaemonSet or Job (extra objects, slower cleanup, more RBAC); nsenter into PID 1 (rejected: keep the image plain, show a `chroot /host` hint in the terminal banner instead).

## R2. Reuse of terminal plumbing
- Decision: generalize `src/backend/exec.rs` session start to take a pod/container target from either a user pod or a node-shell pod, keeping the singleton session, ring buffer and events. Frontend reuses the xterm component from `PodTerminalTab.tsx`, extracted into a shared `TerminalView` if not already.
- Rationale: one attach implementation; avoids a second stream path to test and bound.
- Alternative: separate session manager: duplicates resize/backpressure logic.

## R3. Guaranteed cleanup (layered)
1. Explicit delete on all normal end paths (stop, tab close, exit, supersede).
2. Context-switch and Tauri exit hook: bounded (5 s) delete on the originating client.
3. `activeDeadlineSeconds` (default 4 h): the kubelet stops the containers, but the Pod object stays in `Failed`. So the sweep also treats terminal-phase Fathom pods as orphans.
4. Orphan sweep by label selector `fathom.io/node-shell=true` on connect/open, listed and deleted only after operator confirmation naming each pod.
- Rationale: hard kills cannot run code; the cluster-side deadline bounds exposure and the sweep removes the object. Silent auto-delete of orphans is rejected under Constitution I.
- Alternative: TTL: Pods have no TTL field (`ttlSecondsAfterFinished` is Jobs only).

## R4. RBAC pre-check
- Decision: reuse `rbac_access.rs` `SelfSubjectAccessReview`; add checks for `pods` create/delete/get, `pods/exec` create, `nodes` get. Returns a per-permission result.
- Rationale: identical mechanism to 024, shows exactly which permission is missing. Fail safe on error.

## R5. Failure surfacing
- PodSecurity admission (`restricted`/`baseline`) rejects privileged pods with a Forbidden error: shown verbatim; no bypass. Pending pods: read pod status conditions and container waiting reasons (`ImagePullBackOff`, `Unschedulable`) and show them. Timeout default 90 s.

## R6. Settings persistence
- Decision: additive JSON section under the existing per-context settings in `~/.fathom/` (Constitution VI): missing keys default; invalid file falls back to defaults without deletion.

## R7. Dependencies on 024/025
- 024 supplies the SSAR pre-check pattern and the confirm-before-open UI phases; 025 supplies conventions only. Neither must merge first: the Foundational phase adds the minimal shared piece if absent and notes to dedupe on merge.
- No new crates: `kube` (ws, already enabled), `k8s-openapi`.
