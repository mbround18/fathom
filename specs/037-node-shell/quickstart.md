# Quickstart: validating Node Shell

Prereqs: local kind cluster (`./k8s`), Fathom dev build, an admin kubeconfig context, plus a second context bound to a limited role.

1. Select a node, open Node shell. Confirm the dialog lists node, `default/fathom-node-shell-...`, image, privileges. Cancel: `kubectl get pods -A -l fathom.io/node-shell=true` is empty.
2. Confirm. Observe Creating -> Pending -> Running -> Attached; run `hostname`, `ls /host/etc`, `chroot /host`.
3. Close the tab; verify the pod is gone with kubectl. Repeat with Stop, `exit`, a second Node shell (supersede), context switch, and quitting the app. Each leaves no pod.
4. Start a session then `kill -9` the app. The pod exists; relaunch, open Node shell, see the orphan listed; confirm deletion. Also verify a short `deadline_seconds` (300) moves a pod to Failed.
5. Limited context: open Node shell and see denied permissions listed, no dialog.
6. Label the namespace `pod-security.kubernetes.io/enforce=restricted`, confirm a session, see the verbatim admission error and no pod.
7. Use a nonexistent image: see ImagePullBackOff status; cancel; pod deleted.
8. Check narrow (phone-width) and wide layouts; no horizontal scroll.
9. Tier 1: run steps 1-3 on Ubuntu 26.04, Windows 11, macOS 26.
10. CI gates: `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test`, `pnpm -C ui lint typecheck test`, Playwright.
