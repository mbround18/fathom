# Research: Keel update notice and self-update (spec 061)

Each section settles one question from planning. Code references are to the tree at `cf7fdd6`.

## R1. The registry check

**Decision**: a new shared crate, `crates/oci-registry`, used by Keel (this spec) and Depths (spec
062). It is a HEAD-first OCI distribution client written on `reqwest` 0.13, an adapter with no Keel
or Depths types. Keel calls it from `apps/keel/server/src/updates/check.rs`. The crate's API and
the exact request set are in [contracts/registry.md](contracts/registry.md). Whichever spec is
implemented first builds the crate.

**What the crate provides** (summary; the contract is authoritative):

- `ImageRef::parse`, with Docker Hub defaults: `docker.io` and `index.docker.io` map to
  `registry-1.docker.io`, and a single-segment repository gets the `library/` prefix. A reference
  holds a tag or a digest.
- `Credentials`: `Anonymous`, or `Basic { username, password }` with the password a
  `secrecy::SecretString`. `Debug` redacts the password, and nothing logs it.
- The Bearer token challenge flow, anonymous or with basic credentials, with an in-memory token
  cache until `expires_in` (default 60 s).
- `head_digest(&ImageRef, &Credentials)`: HEAD only. It never falls back to GET. A response without
  `Docker-Content-Digest` is `RegistryError::NoDigestHeader`.
- `resolve(&ImageRef by digest, &Platform, &Credentials) -> Resolved`: GETs the index (if the
  digest names one), then the platform manifest, then the config blob. `Resolved` holds the index
  digest, the platform manifest digest, the config digest, and the config's labels, `User` and
  `created`.
- Every call returns its request counts by method (`RequestCounts { head, get, manifest_get }`),
  so each app feeds its own OpenTelemetry counters.
- A base-URL override, behind the crate's `test-util` feature, so tests point the client at the
  in-process fake registry (an axum router in `oci-registry::testing`). No real network in tests.

**How Keel uses it** (one check, in `updates/check.rs`):

1. **HEAD the tag**: `head_digest(&watched, &credentials)`. Keel uses anonymous Docker Hub by
   default, and `Basic` only when `KEEL_UPDATE_REGISTRY_USERNAME` and the password are configured.
   The digest is the one `docker pull <tag>` records in `RepoDigests`, which is what the role pins
   (R3), because the `Accept` header lists the index and manifest types.
2. **Same digest as running**: the result is `up_to_date`. The check is over, at zero pulls. This
   is the steady state.
3. **Different digest, already in Keel's cache**: the result is reused, again at zero pulls. The
   cache holds what was resolved for each remote digest, for the life of the process.
4. **Different digest, not cached**: `resolve(&watched.with_digest(d), &Platform::current(), &creds)`.
   - The crate picks the index entry for Keel's own `os`/`architecture` (and `variant` for arm),
     skipping attestation entries, or checks a single manifest's config platform.
   - Keel reads `labels["org.opencontainers.image.{version,revision,created,source}"]`, falling
     back to `Resolved.created` for the date.
   - Docker Hub counts up to two manifest GETs as one pull, so each newly published image costs one
     pull, once per process.
   - `RegistryError::NoImageForPlatform` gives `cant_tell` with the reason
     `no_image_for_platform`.
5. **Classify** against the running build (R2):

   | Remote vs running | Outcome |
   |---|---|
   | Same revision, same build date | `up_to_date`: another architecture was rebuilt |
   | Same revision, different build date | `rebuilt` |
   | Remote build date older | `older_build` |
   | Remote build date newer | `update_available` |
   | Either side missing revision or date | `cant_tell` (unless the digest alone is decisive, as in step 2) |

6. **Failures**:
   - The crate maps every failure to a `RegistryError` with a short code: `auth_failed`,
     `not_found`, `rate_limited`, `timeout`, `network`, `bad_response` or `no_digest_header`. Keel
     stores that code in `last_error`.
   - Keel bounds the whole check at 30 s; the crate bounds each request at 10 s.
   - A failure keeps the last successful result (FR-010).
   - There is at most one retry per interval, after 5 minutes. Then the next regular check.
7. **Counts**: Keel adds each call's `RequestCounts` into the check's span and metrics (R9). The
   check's pull count is 1 when `manifest_get > 0`, else 0.

**Rationale**:
- Docker Hub's rules: a GET on `/v2/*/manifests/*` counts as a pull (up to two GETs as one), a
  HEAD does not, and blob GETs do not.
- The only way to read labels is the config blob, and the only way to find the config blob is a
  manifest GET. So "zero pulls" holds while the tag is unchanged, and one pull per newly published
  image is the floor. SC-005 says exactly that.
- One crate for two apps: Depths (062) needs the same HEAD-first check across many images. A
  shared adapter keeps the request set in one place, with one set of request-log tests.
- `reqwest` 0.13.5 is already compiled into keel-server and depths-server through `web-auth`, so
  the crate adds no new dependency edge. The workspace has no `[workspace.dependencies]` table:
  the crate declares `reqwest = { version = "0.13.5", features = ["json", "default-tls"] }`, as
  `web-auth` does. `secrecy` 0.10, `thiserror` 2, `serde_json` 1.0.154 and `sha2` 0.11 are already
  used by sibling crates at those versions.
- In tests, the fake registry is an axum router served on `127.0.0.1:0`. axum is already in the
  lockfile, and is an optional dependency of the crate behind `test-util`. Only test-util clients
  accept a plain `http` base URL; the normal constructor only ever talks `https`.

**Alternatives considered**:
- **A hand-rolled client inside keel-server** (this plan's first draft): Depths would write its own
  second copy. Rejected by the owner in favour of the shared crate.
- **`oci-client` 0.18** (already in the lockfile, used by Fathom's `src/backend/images.rs`).
  Rejected:
  - Its convenience calls (`pull_manifest_and_config`, `pull_manifest_and_config_and_list_digest`)
    always GET the manifest, so even an unchanged tag would count as a pull.
  - Its HEAD (`fetch_manifest_digest`) exists, but mixing it with our own caching and request
    budget means fighting the library's auth state.
  - SC-005 depends on knowing exactly which requests go out, and a small crate of our own makes
    that auditable.
- **hyper-util directly**: no TLS or redirect handling we'd want to write ourselves. Rejected.
- **Docker Hub's REST API** (`hub.docker.com/v2/repositories/…/tags/…`): it gives per-architecture
  digests without counting pulls, but no labels, and it is Docker Hub only. Rejected.
- **Asking the local Docker daemon**: Keel runs distroless and nonroot, with no socket. Mounting
  the socket would be a large privilege grant for a notice. Rejected.
- **Falling back to `ETag` or a GET when `Docker-Content-Digest` is missing**: a GET would count a
  pull for an unchanged tag. Rejected: the crate reports `NoDigestHeader` instead.

## R2. Build metadata

**Decision**: the image tells the binary what it is at run time.

- **Dockerfile, runtime stage**:
  - Keep the `VERSION`, `REVISION` and `SOURCE` args.
  - Add `ARG CREATED=`, and change `ARG VERSION=0.0.0` to `ARG VERSION=`, so an untagged build
    carries no placeholder version (FR-001).
  - Add `LABEL org.opencontainers.image.created=$CREATED`.
  - Add `ENV KEEL_BUILD_VERSION=$VERSION KEEL_BUILD_REVISION=$REVISION KEEL_BUILD_DATE=$CREATED KEEL_BUILD_SOURCE=$SOURCE`.
  - These are in the final stage only, so changing them never invalidates the cargo-chef or
    `cargo build` layers.
- **Binary**: `BuildInfo::from_env()` reads the four variables once at start. An empty or missing
  value is `None`. `KEEL_BUILD_DATE` must parse as RFC 3339, and `KEEL_BUILD_REVISION` as 7–40 hex
  characters with an optional `-dirty`. Otherwise Keel refuses to start, naming the variable: a
  malformed build is a broken image, not a runtime choice.
- **`apps/keel/Makefile`, `image` target**:
  - `VERSION`: set only when `HEAD` has an exact tag `keel-v<semver>`, as
    `git describe --exact-match --match 'keel-v*'` with the prefix stripped. Otherwise empty.
  - `CREATED`: `date -u +%Y-%m-%dT%H:%M:%SZ`.
  - `REVISION`: unchanged (sha, plus `-dirty`).
  - No `keel-v*` tag exists yet, so every image is untagged until the first one is cut (a known
    condition in the spec's Edge cases).
  - `image-push` keeps refusing a dirty tree.
- **Display**, in this order:
  1. a version: `0.2.0`;
  2. no version but a revision: `untagged build abc1234`;
  3. neither: `development build`. This is what `cargo run` and `make keel dev` show, since no
     `KEEL_BUILD_*` is set.

**Rationale**:
- Labels and env come from the same `ARG`, so the registry check (R1) and the running process
  read identical values.
- Runtime env costs nothing to change, and the release build layer stays cached.

**Alternatives considered**:
- **`option_env!("KEEL_BUILD_DATE")` at compile time**: the date changes on every build, which
  would rebuild keel-server from scratch each time and defeat cargo-chef. Rejected.
- **`build.rs` running `git`**: there is no `.git` in the Docker build context. Rejected.
- **Version from `CARGO_PKG_VERSION`**: it is `0.1.0` for every build today, which would mislabel
  untagged builds as a release. It is kept only as the OpenTelemetry `service.version` fallback
  (R9).

## R3. The running digest

**Decision**: `KEEL_IMAGE_DIGEST`, set by the deployment.

- The owner's role (contracts/ansible-role.md) resolves the tag to a digest, pins the compose
  service to `repo@digest`, and writes `KEEL_IMAGE_DIGEST=<digest>` into `keel.env`.
- The digest is the one in `RepoDigests` for the pulled reference: the index digest for a
  multi-arch push, or the manifest digest for a single-manifest push. Either way, it equals what a
  HEAD on the tag returns (R1, step 2).
- It is validated as `sha256:` followed by 64 lowercase hex characters. Anything else stops Keel
  at start, naming `KEEL_IMAGE_DIGEST` (FR-009).

**Fallback when unset**: the digest shows as "unknown" (FR-001).
- With checks on, the comparison skips step 3 of R1 and always classifies by revision and date.
  That means one pull per newly seen remote digest, still at most one per process for each digest.
- The card says "running digest unknown: compared by revision", so the owner knows why.
- The role always sets the digest, so this only happens with hand-run compose files.

**Alternatives considered**:
- **Reading `/proc/self/cgroup` or the Docker socket**: the socket is not mounted, and cgroup
  paths carry the container ID, not the image digest. Rejected.
- **Baking the digest in at build time**: impossible, since the digest is computed after the build
  and depends on the push format. Rejected.

## R4. Running the update with a pinned digest

**Today**:
- The runner reads `.keel/jobs.yaml` from the pinned commit (`apps/keel/runner/src/executor/mod.rs`
  `read_job`), writes `def.vars` to `jobvars.json`, and calls `ansible-playbook` with
  `-e @jobvars.json -e @secret_vars.yaml -e @forced.json`.
- Every variable comes from the signed repository at the pinned commit. The server can't pass a
  value into a run.
- `RawJob` in `crates/keel-core/src/manifest.rs` is `deny_unknown_fields`.

So FR-017 needs a new mechanism.

**Decision**: job parameters, declared by the repository and filled by Keel.

1. **Manifest** (`keel-core`): a job may declare

   ```yaml
   params:
     keel_server_image_digest: { kind: oci_digest }
   ```

   - `kind` is a closed set. This spec adds only `oci_digest`, matching `^sha256:[0-9a-f]{64}$`.
   - At most 8 params per job.
   - Names must be valid Ansible identifiers, and must not start with `keel_internal_` or
     `ansible_`.
   - A param never has a default. When the run doesn't supply it, the variable is simply not set,
     and the play behaves as it does today.
2. **Server**:
   - `NewRun` gains `params: BTreeMap<String, String>`, stored in a new `runs.params jsonb` column
     (empty object by default).
   - The server refuses a param the job doesn't declare, or a value that doesn't match the kind,
     before the run is created.
   - Only server code can set params. No command takes free-form params: the update command (R5)
     is the only caller in this spec.
3. **Frame** (`keel-exec`): `Assign` gains
   `#[serde(default, skip_serializing_if = "BTreeMap::is_empty")] params`.
   - A run without params serializes exactly as today.
   - A run with params is refused by an old runner, because `Frame` is `deny_unknown_fields`. That
     fails closed.
   - When the run's approvals are signed, params join the canonical challenge
     (`OverrideScope` in `crates/keel-trust/src/challenge.rs`) as a `params` member, only when
     non-empty, so existing challenges are unchanged.
4. **Runner**:
   - It re-reads the job from the pinned commit, as today.
   - It refuses the run if the declared params don't match what it was sent: unknown name, wrong
     kind, or a malformed value.
   - Otherwise it writes `params.json` and passes `-e @params.json` after `secret_vars.yaml` and
     before `forced.json`. `forced.json` stays last.
5. **Visibility**: the Runs view shows params next to the commit. The audit entries for the run
   and the update carry them.

**Trust model**:
- The signed repository decides which variables Keel may set, and their shape.
- Keel picks only the value.
- For this job, the role builds the image reference as `<keel_server_image repository>@<digest>`.
  So Keel can choose which image of the configured repository runs, but never a different
  repository.
- A compromised server could still choose an older digest of the same repository. The previewed
  old and new digests are in the audit log, and the role records the previous digest for rollback.
  See Open risks in plan.md.

**Ordering**:
1. Keel and the runner ship params support, and are deployed on Keel's host.
2. Only then does the owner's repo add `params:` to the `keel-server` job. The role change itself
   (contracts/ansible-role.md) can land earlier: it doesn't touch `.keel/jobs.yaml`.

Done the other way round, a manifest with `params` fails to parse on an old Keel or runner
(`deny_unknown_fields`). The run fails closed, but jobs.yaml sync would show the job as broken
until Keel is updated by hand. quickstart.md lists the order.

**Alternatives considered**:
- **Abuse a credential** (a "secret" holding the digest): secrets are for secrets, and they are
  hidden from the run view and audit, which is the opposite of what FR-019 needs. Rejected.
- **A managed inventory with host vars**: signed mode refuses managed inventories
  (`early_check`), and the System workspace is expected to be signed. Rejected.
- **Keel commits the digest to the repository**: Keel would need write access and a signing key
  for the owner's repo. Far too much power. Rejected.
- **The role resolves the tag itself at run time**: the run might deploy a different image from
  the one previewed if the tag moved in between, which breaks FR-017. Kept only as the role's
  behaviour when no digest is passed, which is today's `make keel-server` path.
- **A generic `extra_vars` field on runs**: free-form server-chosen variables would let a
  compromised server override anything in the play. Rejected in favour of repo-declared params.

## R5. Delivering results to owners only, while connected

**Decision**:
- **State**: `AppState.updates: Arc<Updates>`, holding:
  - the `BuildInfo`;
  - the config;
  - a `tokio::sync::watch::Sender<UpdateStatus>`;
  - the remote-digest cache;
  - the time of the last manual check.
- **Checker**: one background task, started in `serve.rs` only when checks are on.
- **Socket**: `ws.rs` adds a `select!` arm on the `watch::Receiver::changed()` future.
  - On change, it re-reads the caller's instance role, from the same per-command check that the
    `pushed` arm uses. Owners get `Event::UpdateStatus` (full). Admins get
    `Event::UpdateStatusLine` (nothing about the available image). Users get nothing.
  - A socket that connects later gets the current value in reply to `GetUpdateStatus`, which the
    web sends after `Me`.
- **Commands**: see contracts/protocol.md.
  - `GetKeelBuild`, for anyone signed in.
  - `GetUpdateStatus`, for owners and admins; the reply is shaped by role.
  - `CheckForUpdates`, owners only. It is rate-limited to once a minute per instance, and the
    reply arrives as an ordinary status push.
  - `DismissUpdateDot { digest }`, owners only.
  - `PreviewKeelUpdate { digest }`, owners only, from a session, with step-up.

**Rationale**:
- A `watch` channel holds exactly one current value, which is all the status is.
- Today every push is workspace-scoped and goes through `repos.events`. Update status belongs to
  the instance. Putting it on the workspace bus would mean a fake workspace ID and a role check in
  the wrong place.

**Alternatives considered**:
- **Broadcasting on `repos.events` with a sentinel workspace**: it leaks to anyone with View on the
  sentinel, and it confuses the existing filter. Rejected.
- **Polling from the web**: violates "fast and quiet". Rejected.
- **Putting status in `Me`**: `Me` is per-session and fixed at sign-in, so it wouldn't update
  live. Only the version goes in `Me` (FR-002).

## R6. Dot dismissal storage

**Decision**: one column. In migration `0011_updates.sql`:

```sql
alter table users add column update_dot_dismissed text
    check (update_dot_dismissed ~ '^sha256:[0-9a-f]{64}$');
```

- The dot shows when `status.available.digest` is set and differs from the owner's
  `update_dot_dismissed`.
- `Me` carries the dismissed digest for owners only.

**Rationale**:
- A dismissal is per owner and per digest, and only the latest digest matters. One nullable
  column is the smallest migration.
- There is no preferences store in Keel today, and this spec is not the place to invent one.

**Alternatives considered**:
- **A `user_preferences (user_id, key, value jsonb)` table**: more general, but the general shape
  has no second user yet. Rejected for now. When a second per-user preference arrives, move both
  then.
- **Browser storage**: wouldn't survive a different browser, as US3 expects. Rejected.

## R7. The release-notes URL

**Decision**: the URL is `<source>/releases/tag/keel-v<version>`, shown only when all of these
hold:

- the available image has a version label;
- the image's `org.opencontainers.image.source` is an `https://` URL;
- that source equals the running build's `KEEL_BUILD_SOURCE` (default
  `https://github.com/mbround18/fathom`).

Otherwise the card says "No release notes for untagged builds" when there is no version, or "No
release notes link: the image's source differs from this build's" when the sources differ.

The link renders with `rel="noopener noreferrer"` and `target="_blank"`. The server builds the URL;
the web never concatenates.

**Rationale**:
- The source label comes from whatever was pushed to the watched tag. Requiring it to match the
  running build's source stops a pushed image from sending owners to an arbitrary site.
- `keel-v` keeps Keel's tags apart from Fathom's `v*` tags in the same repository.

**Alternatives considered**:
- **A configured release-notes URL template**: one more setting, for a value that is already in
  the labels. Rejected.
- **Fetching the release page to check it exists**: an extra outbound request, to a second host,
  for every check. Rejected (FR-004 keeps contact to the registry).

## R8. Configuration

**Decision**: these settings sit alongside the others in `config.rs`. The full list is in
[contracts/config.md](contracts/config.md).

| Variable | Flag | Default |
|---|---|---|
| `KEEL_UPDATE_CHECK` | `--update-check` | `false` |
| `KEEL_UPDATE_INTERVAL` | `--update-interval` | `6h` (minimum `1h`) |
| `KEEL_UPDATE_IMAGE` | `--update-image` | `docker.io/mbround18/keel:latest` |
| `KEEL_UPDATE_REGISTRY_USERNAME` | `--update-registry-username` | unset |
| `KEEL_UPDATE_REGISTRY_PASSWORD` / `_FILE` | none (secret) | unset |
| `KEEL_IMAGE_DIGEST` | `--image-digest` | unset |

- `KEEL_UPDATE_CHECK` uses `parse_bool`. Booleans already read this way, and a second source
  (`off|registry`) has no user yet.
- `KEEL_UPDATE_INTERVAL` uses `parse_duration`.
- The `KEEL_BUILD_*` variables (R2) are set by the image, not by operators. They have no flags.
- The first check runs 60–120 s after start, chosen at random. Later checks run every interval,
  with ±5% jitter, so a fleet of Keels doesn't hit the registry in step. In tests, the random
  source is seeded.

**Rationale**: the existing resolver gives precedence (flag, then env, then default), records
sources, refuses invalid values by setting name, and keeps secrets env- or file-only.

**Alternatives considered**: a settings-table toggle in the UI. Rejected, because FR-004 puts the
switch in deployment configuration, and an outbound-connection switch belongs with the operator,
not with whoever holds an owner session.

## R9. Telemetry

**Decision**: OpenTelemetry, through the existing setup in `apps/keel/server/src/telemetry.rs`. It
exports only when `OTEL_EXPORTER_OTLP_ENDPOINT` is set, as today.

- **Span** `keel.update_check`, from `tracing::info_span!`, with fields filled at the end:
  - `keel.update.trigger` (`scheduled` or `manual`);
  - `keel.update.outcome`;
  - `keel.update.requests.head` and `keel.update.requests.get`, from the crate's `RequestCounts`;
  - `keel.update.pulls` (0 or 1: 1 when `RequestCounts.manifest_get > 0`);
  - `http.response.status_code` of the last request;
  - `error.type` on failure.
  - The watched reference is recorded without credentials. Tokens, passwords and `Authorization`
    headers are never recorded.
- **Metrics**, as `static LazyLock` instruments from `opentelemetry::global::meter("keel")`,
  following the Depths pattern:
  - `keel.update.checks` (counter, attribute `outcome`);
  - `keel.update.check.duration` (histogram, ms);
  - `keel.update.requests` (counter, attribute `method`: `HEAD` or `GET`), fed from each call's
    `RequestCounts`;
  - `keel.update.pulls` (counter). This one lets an operator verify SC-005 from their own metrics.
- `oci-registry` itself records no metrics: it returns counts, and each app names its own
  instruments.
- **Resource**: add `service.version`, from `KEEL_BUILD_VERSION`, else the revision, else
  `CARGO_PKG_VERSION`.
- The update run itself is an ordinary run, with the existing run telemetry.

**Alternatives considered**: metrics through logs only. Rejected, because SC-005 is easiest to
check as a counter.

## R10. Who sees what on the System tab

**Before this spec**: spec 060 made the System tab owner-only (`features/settings/tabs.ts`).

**Spec 061 says**: admins see the running build (FR-002) and the read-only update line (FR-015).
FR-024 opens the tab to admins and records this as an amendment to spec 060.

**Decision**: open the System tab to admins, with role-shaped content:

| Section | Owner | Admin | User |
|---|---|---|---|
| Build panel (version, revision, build date) | yes | yes | no tab |
| Running digest | yes | yes (FR-002) | no |
| Update card (available image, actions, Check now) | yes | no | no |
| Read-only update line ("A newer Keel is available; an owner can update it") | no | yes | no |
| System workspace setup, configuration, binding, host, runs, Remove (spec 060) | yes | no | no |

Users see the version only, under About in the account menu, from `Me.keel_version`.

**Keeping 060's sections owner-only**:
- The web renders the spec 060 sections only for owners, and never sends `GetSystem` or any
  System workspace command for an admin.
- The server is unchanged for those commands: `GetSystem`, `PreviewSystemConfigure`,
  `PreviewSystemRemove` and every System workspace read stay `owner_only` for admins. A server test
  asserts it again after this spec.
- An end-to-end test signs in as an admin, opens the System tab, and asserts that no System
  workspace section, host, run or action renders.

**SC-006**: admins may see the running build, digest included, but never the available update's
digest, details or actions. `UpdateStatusLine` therefore carries no version, revision, date or
digest of the available image.

**Alternatives considered**: a separate "About Keel" tab for admins. Rejected: one more tab for
three lines.
