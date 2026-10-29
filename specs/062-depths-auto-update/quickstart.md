# Quickstart: validating 062

These are run by the main session, one at a time, with `CARGO_BUILD_JOBS=4`. Subagents run only
the targeted commands in §1. Nothing here runs `scripts/ci.sh`, paws or dagger.

## 1. Targeted tests (no Docker, no network)

```
cargo test -p depths-core updates::      # references, label vectors (38, SC-006), effective policy,
                                         # windows with fake clock + DST zone, check/resolve over the
                                         # fake registry incl. the 7-day request budget (SC-007),
                                         # holds (root escalation), keep-tag names, state transitions
cargo test -p oci-registry               # shared with 061: reference parsing, token flow (anonymous, basic),
                                         # HEAD-only head_digest (NoDigestHeader, no GET sent), resolve
                                         # (index, platform pick, attestations skipped, digest mismatch),
                                         # request counts, redacted secrets; against the in-process axum
                                         # fake registry (`test-util`) via with_base_url, no real network
cargo test -p depths-protocol            # round trips for new variants; ts-rs bindings regenerated
cargo test -p depths-server updates      # store (corrupt files), creds (helpers refused, no secret in
                                         # captured logs), scheduler (slots, window close mid-batch,
                                         # Watchtower pause), runner (busy/stale skip, failure doesn't
                                         # stop the batch, health: healthy / unhealthy / restart loop /
                                         # missing replica, rollback, rollback_failed, interrupted),
                                         # audit (Depths (scheduled) + on_behalf_of), paused policy
                                         # when the author is no longer admin, gates, port adapter
                                         # error mapping and reference-parser agreement
pnpm --filter @companion/depths-web test
pnpm --filter @companion/depths-web e2e --project us9-update-check     # states, filters, badge, user sees only own stacks, checks-off empty state
pnpm --filter @companion/depths-web e2e --project us10-update-batch    # batch preview, step-up, per-stack progress, failure summary, hold
pnpm --filter @companion/depths-web e2e --project us11-update-auto     # policies, window editor (host vs stack), rolled back, interrupted
pnpm --filter @companion/depths-web e2e --project us12-watchtower      # label reasons, Watchtower banner + acknowledgement
pnpm --filter @companion/depths-web e2e --project us13-image-cleanup   # cleanup setting, removal entries
```

Fake time: the server tests use the injected `Clock` and `LocalZone`; the DST fixture zone has a
spring-forward gap at 02:00–03:00 and a fall-back repeat at 02:00–03:00. Cases: window 02:30–04:00
on both days, window 23:00–01:00 across both, clock jumping back an hour inside a window.

## 2. Against a real daemon (opt-in)

```
DEPTHS_IT_DOCKER=1 cargo test -p depths-server --test updates_docker
DEPTHS_IT_DOCKER=1 cargo test -p depths-server --test update_walkthrough   # §3 steps 1–5 and 7
DEPTHS_IT_DOCKER=1 cargo test -p depths-server --test update_watchtower    # §3 step 6
```

Run them one at a time, never together: `update_watchtower` starts a Watchtower container, and
Watchtower detection reads every container on the daemon, so it would pause the Auto queues of a
walkthrough running beside it. Each target removes what it created when it ends (its Watchtower
too), so running them back to back is safe.

`updates_docker` starts a `distribution` registry and scratch stacks named `depths-it-upd-<random>`, and
removes everything it created, by name (containers, its own images and tags, the registry). It
never touches other containers, volumes or images. It checks:

- unchanged tag: HEAD only (registry access log), state up to date; the check reaches the registry
  through `Client::with_base_url`, the pull through Docker as usual;
- pushed new image: one resolution, state available with version label;
- multi-arch re-push with the same host image: no update;
- update: only the changed service recreated; compose file byte-identical (hash before/after);
- image whose healthcheck fails: rolled back within timeout + 1 min (SC-004), tag points to the
  old image, `depths-keep/<stack>:<svc>-held` present, digest rejected, next scheduled pass skips it;
- image that turns root: held, tag unchanged, container unchanged (SC-005);
- compose creates the replacement before stopping the old container (R3 risk);
- cleanup on, two updates: only the image from two updates ago removed; an image used by another
  container kept;
- the same run with the containerd image store when the daemon uses it (`docker info` reports
  `driver-type io.containerd.snapshotter.v1`); skipped with a note otherwise.

## 3. End to end on the dev compose

1. Add `/etc/localtime:/etc/localtime:ro` to `depths` in `apps/depths/compose.yaml` (the change
   this feature ships) and bring Depths up as usual.
2. Updates → turn checks on (passkey). Check now. Every stack lists; a pinned-digest stack shows
   excluded.
3. Run a local registry stack, push `app:1`, deploy a stack on it, push a new `app:1`, Check now:
   "update available" with its version.
4. Select it and one up-to-date stack → Update selected → preview lists one service → confirm.
5. Set the stack to Auto with a window covering now; push an image whose command is `false`; wait:
   rolled back, audit shows `Depths (scheduled)` on behalf of you.
6. Start `containrrr/watchtower` (monitor-only) beside it: banner; Auto paused until acknowledged.
7. Restart Depths during step 5's health wait: the stack shows interrupted with its real image.

The same steps run as tests against a real daemon, one command after the other (§2):
`DEPTHS_IT_DOCKER=1 cargo test -p depths-server --test update_walkthrough` (steps 1–5 and 7, and
a restart inside an Auto window that does not run the stack again), then
`DEPTHS_IT_DOCKER=1 cargo test -p depths-server --test update_watchtower` (step 6).

## 4. Steam (production) upgrade notes

- Nothing changes on upgrade: checks are off until an admin turns them on (SC-001).
- To use host time for windows, add the `/etc/localtime` mount.
- **Private registries:** mount a plain `config.json` and set `DOCKER_CONFIG`
  (contracts/config.md, "Private registries"). Credential helpers (`credsStore`/`credHelpers`) are
  not supported; those registries show "can't check". 050's Pull of private images has the same
  gap today, and the same mount fixes both.
- `docker images` will list `depths-keep/<stack>` tags after the first update. They are the way
  back for rollback; don't remove them by hand.
- If Watchtower runs on the host, set it to monitor-only or stop it before switching stacks to
  Auto; see docs/guides/depths/migrating-from-watchtower.md.
- Brochure screenshots of the Updates page are the last step of the batch.
