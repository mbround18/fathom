# Quickstart: In-Cluster Daemon (kind)

Prereqs: docker, kind, kubectl, and `k8s/` + `deploy/Dockerfile` from spec 031 present.

```bash
k8s/cluster.sh up-daemon          # kind cluster + builds/loads fathom-daemon image + applies overlays/daemon + samples/daemon-events.yaml
kubectl -n fathom-system rollout status deploy/fathom-daemon
kubectl get apiservice v1.daemon.fathom.dev   # Available=True
kubectl get --raw /apis/daemon.fathom.dev/v1/v1/capabilities | jq .
```

## Verify Story 1 (replay)
1. `kubectl get --raw /apis/daemon.fathom.dev/v1/v1/stream --request-timeout=5s` shows `hello` then events.
2. Trigger events: `kubectl -n demo set image deploy/web web=nginx:1.27`, `kubectl -n demo create job fail --image=busybox -- false`.
3. In Fathom: Settings -> context -> "Use in-cluster daemon if present", connect; the "While you were away" timeline lists the rollout and job failure.
4. Disconnect, repeat step 2, reconnect: only new events appear.
5. `kubectl -n fathom-system rollout restart deploy/fathom-daemon` -> Fathom shows the truncation banner and resyncs.

## Verify Story 2 (zero footprint)
`kubectl delete -k k8s/overlays/daemon`, reconnect: status shows "Daemon: not installed", all views normal. With preference `off`, confirm no requests via `kubectl get --raw /metrics | grep fathom-daemon` shows none from the client.

## Automated
`k8s/test-daemon.sh` (cluster up, events, replay, truncation, uninstall cleanliness) and `cargo test -p fathom-daemon`.

## Uninstall
`kubectl delete -k k8s/overlays/daemon` — verify `kubectl get apiservice,clusterrole,clusterrolebinding -l app.kubernetes.io/name=fathom-daemon` is empty.

## Verify per-user filtering (FR-017)
Create two users/contexts (one with list access to `demo` only, one to `demo2` only, via RoleBindings). Each sees only its own namespace's events, topology nodes and edges from the same daemon; a user with no access sees `noAccess`. Revoke the RoleBinding mid-session and confirm the entries disappear within 60 s.

## Verify Story 5 (topology and trends)
Scale `deploy/web` and cordon a node: the topology view updates within 5 s. Reconnect after a pause and confirm the trend ring shows the gap or a truncation marker. With metrics-server absent, the UI shows `metricsUnavailable`.

## Note on access
Access is gated by cluster RBAC on the `daemon.fathom.dev` API group and filtered per user inside the daemon. Clusters that forbid `APIService` registration cannot use the daemon.
