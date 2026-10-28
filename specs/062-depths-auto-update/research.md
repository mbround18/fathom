# Research: Depths image updates across many stacks

Decisions for [plan.md](plan.md). Each one names what it settles, why, and what was rejected.
Grounded in the code as of 2026-10-28: `apps/depths/server/src/ops.rs` (Preview → Confirm, one op
per stack), `crates/depths-core/src/{plan,ports,security,compose_lint,role}.rs`,
`crates/depths-runtime-cli`, `apps/depths/compose.yaml`, `apps/depths/Dockerfile`.

## State of the prerequisites

062 builds on four earlier specs. Not all of them are built.

| Spec | What 062 uses | State in the tree |
|------|---------------|-------------------|
| 050 | Pull/Up, `Ops` one-op-per-stack lock, audit log, roles | Built (89 tasks done, 9 open, none on this path). |
| 051 | `security::profile`, `is_root_user`, `compose_lint::lint` | Built in code (`compose_lint.rs`, `security.rs`); tasks.md not fully ticked. |
| 052 | Network trust gate on commands | Built. |
| 054 | Passkey step-up (`Preview.step_up`, `Confirm.assertion`), runtime guard, `depths-guard-core` | Protocol types only. No guard, no server passkeys, no `depths-guard-core`. |

062 therefore lands after 054's step-up (FR-011, FR-017 need it). The guard pre-check (FR-010) uses
`depths-guard-core` when it exists; until then the root check (R4) holds what 051 can see, and a
guard refusal at create time is caught as a hold (R4). See the plan's sequencing.

## R1. Registry check: how to tell a service has a newer image without pulling

**Decision:**

1. Resolve the service's reference from the compose file (`compose config --format json`), not the
   container: `image: nginx` normalises to `docker.io/library/nginx:latest` with Docker's rules
   (own small parser in `depths-core`, table-tested).
2. Find what the container runs: container `.Image` (image ID) and that image's `RepoDigests`
   (`docker image inspect`), one inspect per distinct image ID per check.
3. `HEAD /v2/<repo>/manifests/<tag>` with the four manifest media types in `Accept` (OCI index,
   OCI manifest, Docker manifest list, Docker v2 manifest). Take `Docker-Content-Digest` as `D`.
   HEAD requests do not count against Docker Hub's pull limit.
4. If `<repo>@D` is in the running image's `RepoDigests`: **up to date**. No further request. This
   is the path every unchanged service takes on every check, so a week of checks on an unchanged
   host makes HEAD requests only (SC-007).
5. Otherwise resolve `D` once (the "resolution"), and cache it by digest forever (content
   addressed, so it never goes stale):
   - `GET` the manifest at `D`. If it is an index, pick the entry for the host platform (`os`,
     `architecture`, `variant` from `docker info`, or the service's `platform:` when set) and `GET`
     that platform manifest. Docker Hub counts up to two manifest GETs as one pull.
   - `GET` the config blob (blob GETs are not counted). Read `config.User`,
     `config.Labels["org.opencontainers.image.version"]` and `["…revision"]`, falling back to the
     index/manifest annotations of the same keys.
   - Result: `{ index_digest?, manifest_digest, config_digest, user, version, revision, created }`.
6. Compare by the image the host would run (edge case "same image, new digest"):
   - Classic image store (graphdriver): the image ID **is** the config digest. Same config digest
     → no update (record `D` as an alias so the next HEAD short-circuits).
   - containerd image store (Docker 29 default on new installs): the image ID is the index (or
     manifest) digest the image was pulled by. Compare platform manifest digests. The running
     side's platform manifest is resolved the same way from its own digest, once, cached.
   - The store is detected per host, not configured: under containerd the running image ID equals
     one of its `RepoDigests` digests; under the classic store it never does.
7. Tag moved back: `D` equal to a digest this service ran before (kept in the service's history,
   R5) or to a rejected digest → state `moved_back`, never applied automatically.

**Cost:** an unchanged tag costs one HEAD per check. A newly published image costs at most one
counted pull per registry, **once**, shared by every service and stack that uses it. SC-007 holds
for unchanged tags; the spec's wording ("zero pulls") is stricter than any design that also reads
version labels (FR-003) can meet. See the plan's "Spec notes".

**Why:** this is what Watchtower does (HEAD, compare with `RepoDigests`) plus a cached resolution
so the host-architecture comparison and the version labels are correct without pulling layers.

**Rejected:**
- Daemon `GET /distribution/{name}/json`: it does a manifest GET on every call (counts as a pull
  every check) and needs the socket proxy's `DISTRIBUTION=1`, which the documented setup leaves off.
- `docker manifest inspect` / `docker buildx imagetools`: GETs every time, and not in the image.
- `docker compose pull --dry-run`: still contacts the registry with GETs and reports nothing
  usable for "is it newer".
- Treating any new `D` as an update without resolving: reports false updates for multi-arch
  re-pushes and has no version labels (FR-003).

### Registry client

**Decision (needs owner approval):** reuse `oci-client` 0.18, already in the workspace lock (Fathom
spec 010, `src/backend/images.rs`), behind a `Registry` port in `depths-core`. The adapter is a new
crate `crates/depths-registry-oci`. It brings the bearer-token challenge flow, Docker Hub's
`index.docker.io` mapping and redacting `RegistryAuth` for free.

One behaviour to guard: `Client::fetch_manifest_digest` falls back to a `GET` when a registry
omits `Docker-Content-Digest` on HEAD. Docker Hub, GHCR, Quay, ECR, GAR and `distribution` all send
it, but the adapter must not count a silent GET as a "HEAD". The adapter uses
`fetch_manifest_digest` and records `depths.updates.registry_requests{method}` from what was asked;
the integration test against `distribution` asserts HEAD-only for unchanged tags. If a registry
without the header turns up, the adapter is the one place to change.

**Alternative (no new dependency):** hand-written client on `reqwest` 0.13, already in Depths'
tree through `web-auth`: HEAD/GET with `Accept`, the `WWW-Authenticate: Bearer realm=…,service=…,
scope=…` token dance, basic auth, Docker Hub mapping. About 400 lines plus tests; full control over
HEAD-only. Rejected as the default only because token flows differ subtly between registries and
`oci-client` already handles them in this workspace. The port makes swapping either way cheap.

**Rejected:** `bollard` (not in the tree, and the daemon cannot do HEAD-only checks anyway);
`docker_credential` crate (not needed, see below).

### Credentials

**Decision:** the Docker daemon holds no registry credentials; the CLI does. `docker compose pull`
inside the Depths container already reads `$DOCKER_CONFIG/config.json` (default
`$HOME/.docker`, i.e. `/var/lib/depths/.docker` in the data volume today). The check reads the same
file, so a check and the pull that follows always use the same credentials:

- `auths.<host>.auth` (base64 `user:pass`), `username`/`password`, `identitytoken` (sent as the
  refresh token in the token request). Docker Hub's key is `https://index.docker.io/v1/`.
- `credsStore` / `credHelpers`: helper binaries are not in the image. The service shows
  `check_failed: credential helper "<name>" not available in the Depths container`, and the guide
  explains how to mount a plain `config.json` instead.
- Documented setup: mount a read-only `config.json` at `/run/depths/docker/config.json` and set
  `DOCKER_CONFIG=/run/depths/docker`.
- Values are read per check, held as `secrecy::SecretString`, never logged, audited, put in a span,
  sent over the protocol or written to Depths' state. The only thing shown is the registry host and
  "credentials: from docker config" or "anonymous".
- Anonymous first, credentials on 401, as Fathom's `images.rs` does, so public images on a
  configured registry don't spend authenticated quota.

## R2. The update itself

**Decision:** a new plan operation, `Operation::ImageUpdate { services, pins }`, built per stack
from the check results at preview time:

1. For each distinct new image: `docker pull <repo>@<D>` (with `--platform` when the service sets
   `platform:`). Pulling by digest pins exactly what the preview showed. A tag that moves again
   between preview and confirm cannot change what is installed, and the local tag has not moved yet.
2. Inspect the pulled image (R4). Hold here if needed. Nothing on the host has changed except one
   more image.
3. Remember each service's current image ID (the way back), then tag:
   `docker image tag <new image id> <compose reference>`.
4. `docker compose up -d --no-deps --pull never <services…>` in the stack directory, with the same
   `--env-file` handling as every other plan (`plan.rs`). Compose recreates a container only when
   its image or config changed, so only the named services move. `--no-deps` stops it touching
   dependencies; `--remove-orphans` is deliberately not passed (this is a partial operation).
5. Health wait (R3), then cleanup (R7).

The compose file is never written (FR-009, FR-022). Every step is an argv in the plan, shown in the
preview, run by the existing `CliRuntime`, streamed and audited like 050's Update. The stack's
`content_hash` is bound into the preview, so an edit between preview and confirm refuses as stale.

Shared image within a stack: one pull, all its services in the one `up`. Replicas: `up` recreates
all of them; the health wait checks each.

**Why:** reuses compose's own recreate logic and the CLI runtime; nothing new talks to the daemon.

**Rejected:**
- `docker compose pull <services>` then `up` (what 050's Update does): moves the local tag before
  any check, so a held image is one manual Up away from running, and the image pulled can differ
  from the one previewed.
- A generated override file (`-f compose.yaml -f override.yaml` with `image: …@digest`): changes
  the project's `config_files` label, which Dockge and 050 read, and the next ordinary Up would
  silently go back to the tag.

### Socket proxy and guard

`docker pull`, `image tag`, `image inspect` and `image rm` all go through `/images/*`, already
allowed by the socket proxy's `IMAGES=1` + `POST=1`. 054's guard allowlist lists image create,
inspect and remove but not `POST /images/{name}/tag`; 054's contract needs that line before 062
ships (noted in contracts/protocol.md).

## R3. Health wait and rollback (the hard part)

### Health

**Decision:** after `up`, for every container of every recreated service (all replicas), poll
`docker inspect` every 2 s, only during the wait:

- Has a healthcheck: success when `State.Health.Status == healthy`; fail early on `unhealthy`.
- No healthcheck: success when `State.Running` stays true **and** `RestartCount` stays unchanged
  for the whole timeout (Assumption: "running for the timeout"). A container in a restart loop is
  "running" at most instants; the restart count catches it.
- Any container missing, exited, or not yet healthy when the timeout (default 120 s, setting
  `health_timeout`) passes: fail.

Polling is bounded to the wait; nothing polls at idle (Principle V). `docker events` was rejected
as the primary signal: the events stream through the socket proxy reconnects and can drop, and the
wait must end with a definite answer.

### Rollback

**Decision: retag the previous image locally, then recreate.**

1. Before step 3 of R2, the previous image ID of each service is already kept (R7's keep tag
   `depths-keep/<stack>:<service>-previous`), so no prune can remove it between pull and rollback.
2. On health failure: `docker image tag <previous id> <compose reference>` then
   `docker compose up -d --no-deps --pull never <failed services…>`. Compose sees a different
   image ID for the tag and recreates onto the old image.
3. Health-wait again with the same rules. Success → `rolled_back`. Failure → `rollback_failed`,
   which the Updates page shows in red with the output, and the stack stays as it is for a human.
4. Record the new digest as a rejected digest for those services (FR-015).

**What the host looks like after a rollback:** the compose reference's local tag points to the
old image again, so a manual Up or restart by anyone keeps the old, working image. The bad image is
kept under `depths-keep/<stack>:<service>-held` for inspection until the next successful update or
a manual retry. The registry still points the tag at the new image, so the next check reports
"update available, rejected" rather than up to date.

**Why this over the alternatives:** it is the only option that leaves the stack consistent for the
next ordinary compose command. The container is still created by compose from the compose file,
with its labels, networks and name, so 050, Dockge and `docker compose ps` all see a normal stack.

**Rejected:**
- Override file pinning `image: <previous id>`: the next plain Up without the override goes back to
  the tag, which still points at the bad image; and the `config_files` label changes.
- `docker run`/`docker create` from the old container's config: the container stops being a
  compose container (labels, network aliases, `depends_on` ordering).
- Keeping the old container (stop and rename) and swapping back: compose owns container names and
  would fight the rename on the next `up`; also doubles disk use of anonymous volumes.
- Pull-by-previous-digest from the registry: the old digest may be gone upstream, and it costs a pull.

### Risks that stay

- **Data migrations** are not undone (Assumption). A new image that migrates a database and then
  fails its healthcheck is rolled back onto a database the old image may not read. The guide says
  to keep such stacks on Notify. Depths cannot detect this.
- **Shared tags across stacks:** tags are host-wide. Rolling back stack A retags `app:1` to the
  old image. Stack B, already updated to the new `app:1` image in the same window and healthy,
  keeps running it, but would get the old image on its next recreate. The Updates page shows B as
  "update available" again. This is the same exposure `docker compose pull` has today; documented.
- **Rollback itself fails** (old image now fails too, e.g. after a migration): `rollback_failed`,
  stop, show output; no third attempt.
- **containerd image store tagging:** `docker image tag <index digest id> ref` is expected to work
  the same way; the opt-in Docker integration test runs both stores (quickstart §2).
- **Compose create-before-stop:** compose v2 creates the replacement container under a temporary
  name before stopping the old one; a guard refusal at create therefore leaves the old container
  running. This is relied on in R4 and must be confirmed by the integration test, not assumed.

## R4. Holding root escalation and guard refusals before recreate

**Decision:** between pull and tag (R2 step 2), for each service:

1. **Root escalation (FR-010):** if the compose service sets `user:`, the image's user does not
   matter; no hold. Otherwise compare the running container's `Config.User` (the effective user it
   runs as today) with the pulled image's config `User` via `security::is_root_user`. Hold when the
   old one is not root and the new one is, and the daemon mode makes that a `Level::Root` finding
   (rootful). On rootless/userns daemons 051 rates it `Info` (`RootRemapped`); shown, not held.
2. **Escape findings:** re-run `compose_lint::lint` on the current compose text. The compose file
   does not change in an update, so the lint result cannot change; it is re-run so that an Auto
   stack whose file was edited since the policy was set is judged on its current text. Escape
   findings under `DEPTHS_COMPOSE_POLICY=enforce` refuse, as for any deploy; under `warn` the batch
   preview shows them (the admin's confirm acknowledges them, as 051's "deploy anyway"), and Auto
   holds any stack with an escape finding the admin has not acknowledged.
3. **Guard (054):** when `depths-guard-core` exists, judge the service's effective create request
   (compose config + new image config) with it and hold on refusal. Before it exists, a guard
   refusal surfaces from `up` as 054's `GuardRefusal`; because compose creates before it stops
   (R3 risks), the old container keeps running, and the stack is recorded `held` with the rule.

A hold leaves the host unchanged: the tag never moved (R2), and the pulled image is kept as
`depths-keep/<stack>:<service>-held` for review. "Apply anyway" is an admin action with passkey,
audited with the finding (US2 scenario 3, SC-005).

**Why:** the image's `User` is the only thing that changes in a tag update; the 051 lint reads the
compose file and cannot see it (`compose_lint` flags a service without `user:` as root-level
regardless of image). Comparing the effective user before and after is exact.

**Rejected:** running the new container first and inspecting it (already too late for a hold);
holding every service without `user:` (would hold almost every stack on the first update).

## R5. Scheduling and state

### Time and windows

**Decision:**
- A window is `{ days: [mon..sun], start: "HH:MM", end: "HH:MM" }` in host local wall time.
  `end <= start` means it crosses midnight. Per-stack windows replace the host window.
- The core evaluates windows through a small `LocalZone` trait (`to_local(utc) -> NaiveDateTime`,
  `from_local(naive) -> LocalResult<DateTime<Utc>>`), and a `Clock` (`now() -> DateTime<Utc>`),
  both injected, like `file_history`'s `Clock`. The server adapter is `chrono::Local`; the `clock`
  feature is already on in the server. No timezone crate.
- DST:
  - **Skipped hour** (start falls in the gap): the window starts at the first valid instant after
    the gap (`LocalResult::None` → step forward). The day is not skipped.
  - **Repeated hour** (start is ambiguous): the earliest instant (`LocalResult::Ambiguous(a, _)`).
    Each window occurrence is keyed by the local date it starts on; one occurrence runs once.
  - End resolves the same way. An occurrence whose end resolves before its start is empty, and the
    next occurrence is used.
- Clock jumps: the loop recomputes from `now` each tick; occurrence keys stop a backwards jump
  from running a window twice.
- Host local time inside the container: the image (`debian:trixie-slim`) has no `tzdata`, and the
  container has no `/etc/localtime`, so `chrono::Local` is UTC today. The documented compose adds
  `/etc/localtime:/etc/localtime:ro`; chrono reads it directly. When the zone is UTC, the settings
  page says "windows are in UTC: mount /etc/localtime to use host time".

### The loop

**Decision:** one background task (`updates::Scheduler`, spawned in `main.rs` beside `Refresher`):
- Sleeps until the next due thing: the next check slot, or the next window opening. No fixed tick
  at idle (Principle V). Woken early by a settings change or Check now (`tokio::sync::Notify`).
- **Checks** are spread across the interval: each stack gets a slot
  `hash(stack) mod interval`, so 30 stacks every 6 h are ~12 min apart, never a burst. Check now
  runs everything once, rate-limited to one per minute host-wide (FR-002).
- **Backoff** per registry host: after a 429 or 5xx, exponential from 1 min up to the check
  interval, honouring `Retry-After`. Services on that registry show `check_failed` with "rate
  limited until …" and keep their state (FR-005).
- **Auto runs** in an open window: candidates are Auto stacks with an update available, not held,
  not rejected, not busy; ordered by stack name (stable, documented). One at a time through the
  same `Ops` lock. Before starting each stack, if the window has closed, stop (FR-013); the stack
  in progress finishes.
- Auto is paused while an unacknowledged Watchtower container runs (R6) or `DEPTHS_UPDATES_AUTO=off`.

### State

**Decision:** two files in the data directory, written with `users.rs`'s pattern (temp 0600,
fsync, rename, fsync dir):

- `updates.json`: what administrators decided. Host settings, per-stack policies (with who set
  them and when), rejected digests, Watchtower acknowledgements. Versioned, `deny_unknown_fields`.
  **Unreadable:** Depths starts, treats checks as off and Auto as paused, shows the error and the
  path on Updates, and leaves the file alone (Principle VI). It never resets it.
- `update-state.json`: what Depths observed. Per-service last check result, the digest resolution
  cache, per-registry backoff, the run journal. **Unreadable:** moved aside to
  `update-state.json.unreadable-<ts>` and rebuilt by the next check; it holds nothing a person typed.

Run journal and interruption (FR-018): before a stack's first mutating step, the run writes
`{ run_id, stack, services, previous image ids, started_at, phase }` to the journal and fsyncs;
each phase change updates it; the stack's final outcome closes it. On start, any open entry is
closed as `interrupted`, an audit entry is written with the actor of the run, and the stack's real
state is read from the runtime (which image each service now runs). Depths does not resume or roll
back an interrupted update on its own: the admin sees what is running and decides.

Policies are kept per stack name. A stack directory that disappears keeps its policy (shown
"missing", like users' assigned stacks) so recreating it does not silently make it Auto.

## R6. Watchtower labels

**Decision:** read labels from the running container (Watchtower reads container labels, which
include labels inherited from the image); for a service with no container, from
`compose config --format json`. Exact keys, values and test vectors are in
[contracts/watchtower-labels.md](contracts/watchtower-labels.md). Summary:

| Label | Depths |
|-------|--------|
| `com.centurylinklabs.watchtower.enable` | `false` excludes. With "only labelled services" on, anything but `true` excludes. |
| `…monitor-only` | `true` caps the effective policy at Notify. |
| `…scope` | With a Depths scope set, include only equal scopes; scope `none` matches unlabelled and `none`. With no Depths scope, include all and show the scope. |
| `…no-pull` | `true` excludes ("no-pull by label"): Depths only updates by pulling. Beyond the spec, restrictive. |
| `…lifecycle.{pre-check,pre-update,post-update,post-check}` and `…-timeout` | Shown "not run": hooks are commands inside the container (051 guarded shell). |
| `…depends-on` | Shown "not run: order comes from the compose file". |
| `…stop-signal` | Shown "not used: compose's `stop_signal` applies". |

Booleans follow Watchtower's Go `strconv.ParseBool` (`1 t T TRUE true True 0 f F FALSE false
False`); anything else is treated as absent and shown as "unrecognised value".

Effective policy = the most restrictive of the stack policy and the labels, with the source named
("label: monitor-only", "stack policy") in the UI (FR-019, US4 scenario 7).

**Watchtower detection:** from the existing runtime snapshot (no extra polling): a running
container with label `com.centurylinklabs.watchtower=true`, or whose image repository is
`containrrr/watchtower` (any registry prefix, including `ghcr.io/containrrr/watchtower`) or one of
the known forks (`nickfedor/watchtower`, `ghcr.io/nicholas-fedor/watchtower`, `beatkind/watchtower`;
list confirmed at implementation). An acknowledgement is stored per container ID plus image ID, so a
recreated Watchtower raises the banner again. Depths never stops or changes it (FR-021).

**Rejected:** reading Watchtower's own flags (`--label-enable`, `--scope`) from the Watchtower
container to copy its settings: tempting, but it reads another container's command line and would
silently change Depths' behaviour when that container changes. The admin sets Depths' scope and
label-only mode explicitly.

## R7. Image cleanup and keeping the way back

**Decision:** Depths names the images it is keeping, with local tags in a repository nobody pushes
to: `depths-keep/<stack>:<service>-previous` and `…:<service>-held`. Stack and service names are
lowercased and any character outside `[a-z0-9._-]` becomes `-`, with an 8-hex hash suffix when that
changed the name (collisions). Tags are visible in `docker images` and documented.

- On a successful update, `previous` moves to the image just replaced. That protects the way back
  from `docker image prune` (which only removes dangling images) until the next successful update.
- **Cleanup off (default):** moving the tag is all. The image that was `previous` before is left
  untagged, exactly as a manual pull leaves it today. Nothing is deleted (FR-023, US5 scenario 3).
- **Cleanup on:** after the stack is healthy, `docker image rm <old previous image id>` without
  `-f`. The daemon refuses when any container (running or stopped, in any stack or outside the
  stacks directory) uses it, or when another tag references it; a refusal is a "kept: in use"
  note, not a failure. Each removal is audited with the image ID and repo digests (US5).
- Depths never removes an image it did not record as a service's previous image. No `prune`.
- A rolled-back service keeps `previous` (the image it rolled back to) and `held` (the bad one).

**Rejected:** relying on dangling images (any `docker image prune`, including this dev box's
`dev-gc`, can remove the way back mid-window); `docker image prune --filter` (removes images Depths
never touched).

## R8. "Depths (scheduled)" as an audit actor

**Decision:** add `Provider::Depths` (serialised `depths`) to the protocol's `Provider`, used only
for actors the server creates: `Actor { provider: depths, subject: "scheduled",
username: "Depths (scheduled)" }`. `AuditEntry` gains `on_behalf_of: Option<Actor>`: the
administrator who last set the stack's policy (FR-016). `users.json` validation refuses the
`depths` provider, so nobody can sign in as it. `Busy` refusals name it naturally ("busy: Depths
(scheduled) is updating this stack").

New `AuditAction`s: `image_update`, `image_rollback`, `image_hold`, `image_hold_apply`,
`image_remove`, `update_interrupted`, `update_settings`, `update_policy`, `watchtower_ack`,
`update_retry`. Update entries carry an `update` detail (services, old/new image ID and digest,
version labels, outcome, rejected digest). Settings and policy entries carry before/after
(FR-017). Shapes in [data-model.md](data-model.md).

**Rejected:** a separate `Actor` enum (breaks every reader of `actor.username` in the UI and the
export format); writing the policy author as the actor (says a person did something they did not).

## R9. Telemetry

**Decision:** follow `telemetry.rs`: spans and metrics go nowhere unless
`OTEL_EXPORTER_OTLP_ENDPOINT` is set.

- Spans: `updates.check` (`stack`, `registry`, `outcome`), `updates.resolve` (`registry`,
  `outcome`), `updates.run` (`run_id`, `trigger = manual|scheduled`, `stacks`), `updates.stack`
  (`stack`, `outcome`), `updates.health_wait` (`stack`, `service`, `outcome`, `waited_ms`).
  Never credentials, never paths, never full `auths` keys.
- Metrics (meter `depths`): `depths.updates.registry_requests` counter
  {`registry`, `method` = head|get_manifest|get_blob, `status`}; `depths.updates.check_ms`
  histogram; `depths.updates.outcomes` counter {`outcome`, `trigger`};
  `depths.updates.health_wait_ms` histogram. The GET counter is how an operator verifies SC-007.

## R10. Feature flags

**Decision:** runtime, never compile-time:
- `DEPTHS_UPDATES` = `on` (default) | `off`. `off` hides the feature: no Updates page, no
  scheduler task, no registry client constructed, and update commands answer `Unsupported`. The
  operator's kill switch.
- `DEPTHS_UPDATES_AUTO` = `on` (default) | `off`. `off` pauses every Auto policy host-wide without
  changing them (break glass during an incident). The Updates page says why.
- The admin setting `checks_enabled` (default `false`) is the product switch: with it off, Depths
  makes no registry requests (FR-001, SC-001). Cleanup has its own setting, default off.

Security checks in this feature (hold on root escalation, guard, trust, passkey) have no switch,
as in 054 R13.
