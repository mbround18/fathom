# Quickstart Validation: Port Forward Interface (009)

Prerequisites: `pnpm -C ui install`, a running fake-backend Playwright harness (see
`ui/tests/e2e/` existing specs for the fixture pattern), `cargo build` succeeds.

## US1 — Start a forward from a Pod inspector

1. Open a Pod inspector (`Select a pod → Details`), open the new "Forward" tab.
2. Click "Forward" on a declared container port.
3. Confirm dialog names namespace/pod/container/port; confirm.
4. Assert `StartPortForward` was sent, and the Port Forwards panel (list pane routed to
   `network/portforwards`) shows the new row transitioning `Connecting` → `Active` as the fake
   backend emits `PortForwardsUpdated`/`PortForwardTunnel`.

## US2 — Manage the live list

1. Seed the fake backend with 2-3 `PortForward` rows in different statuses.
2. Open the Port Forwards panel; assert each row shows local port, namespace, pod, remote port,
   status.
3. Stop an `Active` row (confirm dialog), assert `StopPortForward` sent and row doesn't flip to
   `Closed` until the backend's `PortForwardsUpdated` reply arrives.
4. Restart an `Error`/`Closed` row; assert the same confirm+port-check flow as US1 runs and a
   fresh `StartPortForward` is sent for the same namespace/pod/port.
5. Emit a `PortForwardTunnel` status event out of band; assert the row updates without a manual
   refresh.

## US3 — Port picker conflict detection

1. Fake backend responds `available: false` for a specific port via `PortAvailability`.
2. Open the forward-start dialog, edit the local port field to that port, blur.
3. Assert a conflict warning renders and the confirm button is disabled/blocked.
4. Edit to a free port; assert the warning clears and confirm becomes available.

## US4 — Copy address

1. Open the panel with an `Active` row.
2. Click its copy-address action; assert `navigator.clipboard.writeText` was called with exactly
   `localhost:<local_port>` and a toast/confirmation renders.
3. Repeat on a `Connecting` row; assert the tunnel-not-ready state is visibly indicated.

## Verification commands (run all before considering any phase done)

```bash
cargo build
cargo clippy --all-targets -- -D warnings
cargo test
pnpm -C ui typecheck
pnpm -C ui lint
pnpm -C ui test -- --run   # unit
pnpm -C ui exec playwright test tests/e2e/us1-port-forward-start.spec.ts tests/e2e/us2-port-forward-manage.spec.ts tests/e2e/us3-port-forward-port-picker.spec.ts tests/e2e/us4-port-forward-copy-address.spec.ts
```
