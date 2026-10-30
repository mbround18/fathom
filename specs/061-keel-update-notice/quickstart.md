# Quickstart: validating spec 061

## Prerequisites

- A running dev stack:
  - the Keel server against the throwaway test database
    (`eval "$(scripts/keel-test-db.sh start)"`; `make keel test` does this for you);
  - the web dev server.
- An owner account, an admin account and a user account.
- For scenarios 9–13, a System workspace (spec 060) bound to the Ansible repository, with Keel's
  own host enrolled with the `keel-server` profile.

Detailed shapes are in [contracts/](contracts/) and [data-model.md](data-model.md).

## Rollout order

Params (research R4, FR-023) cross two repositories, so the order matters. Keel and the runner
ship first; the Ansible repository adds `params:` last:

1. Ship Keel and the runner with params support (this spec).
2. In the Ansible repository, land the role change (contracts/ansible-role.md): digest pinning,
   `KEEL_IMAGE_DIGEST`, `digest.previous` and the pull fix. It doesn't touch `.keel/jobs.yaml`, so
   it is safe at any point.
3. Update the runner on Keel's host (`make keel-node`).
4. Update Keel by hand once (`make keel-server`). With the role change in, this deploy pins the
   digest and sets `KEEL_IMAGE_DIGEST`.
5. Only now, in the Ansible repository, add the `params:` line to the `keel-server` job in
   `.keel/jobs.yaml`.
6. From then on, the Update button works.

Before step 5, the card shows the manual path with `job_missing_param`. Adding `params:` before
steps 3 and 4 makes the old Keel or runner refuse the manifest (`deny_unknown_fields`): it fails
closed, but the `keel-server` job shows as broken until Keel is updated by hand.

## Targeted tests

Run these one at a time, with `CARGO_BUILD_JOBS=4`:

```sh
cargo test -p oci-registry                    # ImageRef, token flow, head_digest, resolve, counts, against the fake registry
cargo test -p keel-core manifest              # params parse rules
cargo test -p keel-exec frames                # Assign with and without params
cargo test -p keel-trust                      # OverrideScope with and without params
cargo test -p keel-protocol --test roundtrip_061
cargo test -p keel-server updates             # check, cache, classify, schedule, against oci-registry's fake registry
cargo test -p keel-server --test updates      # DB: dismiss, update run, one-at-a-time, audit, keel_updated
cargo test -p keel-runner params              # params.json, mismatch refusal, -e order
make keel bindings                            # then confirm no diff
pnpm --filter @companion/keel-web test        # status reducer, dot rule, tab visibility
pnpm --filter @companion/keel-web exec playwright test --project=s061
```

The unit tests are deterministic:

- The fake registry (`oci_registry::testing::FakeRegistry`, feature `test-util`) runs in-process
  on `127.0.0.1:0`.
- The scheduler runs under `tokio::time::pause`, with a seeded jitter source.
- Time comes from the existing `Clock`.

Nothing reaches the network.

## Playwright

Add a new project, `s061`, to `apps/keel/web/playwright.config.ts`:

```ts
{ name: 's061', testDir: './tests/e2e/s061', use: { ...devices['Desktop Chrome'] } }
```

It runs against the fake backend, like the other projects. The files sit in their own directory,
so the `s060` pattern (`settings-*.spec.ts`) never picks them up. Each spec file can run alone:

| Spec | Covers |
|---|---|
| `update-build.spec.ts` | US1. The build panel for owners and admins, About for users, and "development build" and "untagged build" display |
| `update-notice.spec.ts` | US3. The pushed status, the dot, dismissing it, the card staying, reload persistence, older and rebuilt wording, failure display |
| `update-roles.spec.ts` | SC-006, FR-015 and FR-024. For each role: what renders; that admins see the running build and digest but no available digest, details or Update action; that admins see no spec 060 System workspace section; that users have no System tab |
| `update-run.spec.ts` | US4. Preview, step-up, the run in Runs, a second request showing the existing run, the refusal states |
| `update-manual.spec.ts` | US5. Each missing prerequisite names itself, and the manual steps show the digest reference |

## Scenarios

| # | Do | Expect |
|---|---|---|
| 1 | Start Keel with default config. Watch outbound connections for a while (`ss -tnp`, or the fake registry's log). | No registry request. The System tab says checks are off (FR-004, SC-001). |
| 2 | Start Keel with `KEEL_UPDATE_INTERVAL=30m`. | Refuses to start, naming `KEEL_UPDATE_INTERVAL` (FR-009). |
| 3 | Run `cargo run -p keel-server` with no `KEEL_BUILD_*`. Open System as owner. | "development build". Revision, date and digest are "unknown" (FR-001). |
| 4 | Build with `make keel image` on an untagged commit and run it. | The short revision ("Keel <sha>" under About), the build date, and no version. |
| 5 | Sign in as admin, then as user. | Admin: the System tab with the build panel and digest, no update card, and none of spec 060's System workspace sections. User: no System tab, version under About (FR-002, FR-024). |
| 6 | Turn checks on, pointed at the fake registry, with the same digest as running. | Within 2 min, "Up to date", and the request log shows HEAD only (SC-005). |
| 7 | Publish a newer digest to the fake. Press Check now. | Within 30 s, the dot and card appear without a reload. One manifest GET pair and one blob GET (SC-002). Press again at once: "try again at …" (FR-007). |
| 8 | Dismiss the dot. Reload. Publish another digest and check. | The dot stays gone after the reload and comes back for the new digest. The card stays throughout (FR-014). |
| 9 | Make the fake return 429. Check now. | The card keeps the last result, and shows `rate_limited` and the next attempt (FR-010). |
| 10 | Remove `keel-server` from the System workspace's allowed profiles. | The card shows manual steps with `profile_not_allowed`, and the `mbround18/keel@sha256:…` reference (FR-021). |
| 11 | Restore it. Press Update, then step-up. | A `keel-server` run in Runs with `params.keel_server_image_digest`. Audit has `keel_update_requested` with old and new digests (FR-017, FR-019). |
| 12 | Press Update again while it runs. | The preview names the existing run. Confirm opens it. Audit has `keel_update_refused { update_in_progress }` (FR-018). |
| 13 | Let the run finish. Keel restarts. | Audit has `keel_update_result`, then `keel_updated` with the old and new digest. The build panel shows the new digest (FR-019). |
| 14 | Call `PreviewKeelUpdate` with an API token, and as an admin. | `session_required` and `owner_only`, both audited (FR-020). |
| 15 | Roll back with `make keel-server KEEL_DIGEST=$(cat /opt/keel/digest.previous)`. | Keel runs the old digest, and `keel_updated` is audited on start. |
