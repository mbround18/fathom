# Quickstart: Container Image Inspection

## Validate US1 (public image)

1. `kubectl run nginx-demo --image=nginx:1.27 --restart=Never`
2. Open Fathom, navigate to Workloads → Pods → `nginx-demo`.
3. Open the inspector, select the "Image" tab, pick container `nginx-demo`.
4. Expect: registry `docker.io`, repository `library/nginx`, digest, total size, ordered layer
   list with sizes and history commands — populated without leaving Fathom, within ~5s.
5. Re-open the tab: no new network fetch (verify via backend log / tracing span count for the
   same digest).

## Validate US2 (private registry)

1. Create a `kubernetes.io/dockerconfigjson` Secret for a private registry and a Pod referencing
   it via `spec.imagePullSecrets`.
2. Open that Pod's "Image" tab; expect success with no credential prompt.
3. Break the secret (wrong password) and repeat; expect an explicit "authentication failed" state.

## Validate US3 (base image, best-effort)

1. Open the "Image" tab for the `nginx-demo` pod (or an `alpine:3.19` pod).
2. Expect a best-effort base-image label when layer digests match the built-in table, or an
   explicit "not determined" state otherwise.

## Automated checks

- `cargo test` — protocol round-trip + manifest/config parsing + auth-fallback unit tests.
- `cargo clippy --all-targets -- -D warnings`
- `pnpm -C ui typecheck && pnpm -C ui lint`
- `pnpm -C ui test` (component tests)
- `pnpm -C ui exec playwright test ui/tests/e2e/us1-image-inspection-*.spec.ts`
