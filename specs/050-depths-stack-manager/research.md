# Research: Depths (050)

All unknowns from the plan's Technical Context are resolved here. Sources: a clone of
`louislam/dockge` at main `0329dab` (2026-10-23; latest release 1.5.0, 2025-03-30), compose-go
v2.16.2 / docker compose v5.6.1, the Tecnativa and linuxserver socket-proxy READMEs and
`haproxy.cfg`, GitHub's OAuth App docs, and a read of this repository's container runtime
(`src/backend/container_runtime/`), hosted server (`server/src/`) and UI (`ui/src/`).

## R1. Dockge on-disk compatibility

**Decision**: Depths reads the same layout Dockge does and writes less than it does.

- Stacks directory: `--stacks-dir` / `DEPTHS_STACKS_DIR`, default `/opt/stacks`. Only the
  immediate sub-directories are stacks (Dockge `backend/stack.ts:263-332`). Nested directories are
  not stacks in v1, the same as Dockge.
- A directory is a stack when it holds a compose file. **Depths picks the file Compose picks**:
  `compose.yaml`, `compose.yml`, `docker-compose.yml`, `docker-compose.yaml`, in that order
  (compose-go `cli/options.go:521-525`). Dockge's order differs (`compose.yaml`,
  `docker-compose.yaml`, `docker-compose.yml`, `compose.yml`, `common/util-common.ts:110`), so
  when two exist Dockge can edit a file Compose never runs. When a directory holds more than one,
  Depths shows a warning naming both and the one Compose will use (FR-004).
- Stack name = directory name = compose project name. Lifecycle commands run with
  cwd = `<stacks>/<name>` and no `-f`, exactly like Dockge, so the project is the same one Dockge
  and a terminal see.
- A directory name that is not a valid project name (`^[a-z0-9][a-z0-9_-]*$`, compose-go
  `loader.go:815`) is listed and marked invalid, never acted on. Dockge silently shows such stacks
  as inactive (dockge #737, #931).
- `global.env`: Dockge main (unreleased, PR #387) supports `<stacks>/global.env` by running
  `docker compose --env-file ../global.env [--env-file ./.env] <cmd>`; the explicit `./.env` is
  needed because any explicit `--env-file` disables Compose's automatic `.env`. Depths does the
  same **when `global.env` exists**, and the preview shows the flags. With no `global.env`, Depths
  passes no `--env-file`, which is 1.5.0's behaviour.
- Writes: Depths writes the compose file and `.env` verbatim (the text the operator typed, no
  re-serialisation). Dockge's visual editor rebuilds YAML and has lost comments (#176); Depths has
  no visual editor in v1, so it cannot. Delete keeps the directory by default; Dockge `rm -rf`s it.
- Dockge's own state (SQLite in its data dir) is not read or written. Depths' state (audit log)
  lives in its own data dir, outside the stacks dir.
- Projects from `docker compose ls --all` with no matching directory are foreign: shown read-only
  (Dockge: "This stack is not managed by Dockge.").

**Path identity**: Compose resolves relative bind mounts (`./data`) against the project directory
*inside the Depths container*, then the daemon mounts that path *on the host*. So the stacks
directory must be mounted at the same path on both sides, as Dockge already requires. Depths
checks this at start-up: if a container's `com.docker.compose.project.working_dir` label names a
path under a different root than the stacks dir for a stack it recognises, it logs a warning and
shows a banner.

**Alternatives considered**: a translation layer between container and host paths (it breaks every
relative path in operators' compose files, so rejected); following Dockge's file order (it keeps
Dockge's bug, so rejected).

## R2. Lifecycle commands and status

**Decision**: the same commands as Dockge, with the previews and confirmations from 048.

| Action | argv (cwd = stack dir) |
|---|---|
| up / deploy | `docker compose [env-files] up -d --remove-orphans` |
| stop | `docker compose [env-files] stop` |
| restart | `docker compose [env-files] restart` |
| down | `docker compose [env-files] down` (+ `--volumes` only when chosen, off by default) |
| pull | `docker compose [env-files] pull` |
| update | pull, then `up -d --remove-orphans`, shown as one preview with two commands |
| delete | `down --remove-orphans`, then (only when chosen) remove the directory |
| validate | `docker compose [env-files] -f - config --quiet` (draft on stdin, cwd = stack dir) |
| logs | `docker compose logs -f --tail <n> [svc]`, or `docker logs -f` per container |
| shell | `docker exec -it <container> <shell>` in a PTY (the 048 session code) |

Per-service up/stop/restart use the same `[env-files]` (Dockge skips them there, which is a bug).

**Status**: derived from one `docker ps -a --format '{{json .}}'` (048's `inventory.rs`) grouped by
the `com.docker.compose.project` label, not from `compose ls`'s summary string. Dockge's
`statusConvert` marks a stack "exited" when any container has exited, including one-shot init jobs
(#177, #806, the most upvoted issues). Depths counts a container that exited with code 0 and has
restart policy `no` as **completed**, which does not make the stack partial. Statuses:
`running` (every long-running service up), `partial`, `stopped`, `not deployed` (files, no
containers), `invalid` (unparseable or invalid name), plus a separate `unhealthy` flag when any
health check fails.

**Alternatives**: `compose ls` status strings (they cause the one-shot bug, so rejected); the Docker
Engine API through `bollard` (rejected in 048's research: an argv preview is reproducible and a
socket call is not, and FR-012 needs the exact command).

## R3. Reusing the 048 container runtime: crate boundary

**Decision**: a new library crate `crates/container-runtime` takes the pure Docker/Compose modules
and the wire types they produce. Fathom re-exports both, so Fathom's protocol and generated
TypeScript do not change by one byte (the bindings drift check proves it).

What moves, from the coupling map:

- `Runner`, `CmdOutput`, `RunError`, `SystemRunner` out of `backend/kind.rs`, which then imports
  them back. A `run_with_stdin` method with a default implementation is added for `validate`.
- `stream.rs`, `probe.rs`, `compose.rs`, `stats.rs`, `mutate.rs`, `blast_radius.rs`,
  `inventory.rs`, `grouping.rs`, `preview.rs`, `logs.rs`, `session.rs`, `mod.rs` (`Cli`,
  `parse_size`), and the parts of `adoption.rs` that are not about Fathom's command enum.
- The 37 types in `src/protocol/v1/container_runtime.rs` plus `KindRuntimeKind`, keeping their
  `serde` and `ts-rs` derives. `fathom::protocol::v1` re-exports them (`pub use container_runtime::wire::*`).

Seams cut to make that possible:

- **Event sink**: `logs.rs` and `session.rs` write to Fathom's `log_streams::StreamSink`. They take
  a crate-defined `trait StreamSink { fn emit(&self, StreamEvent) -> bool }` instead. Fathom's
  existing sink implements it, and Depths' WebSocket fan-out implements it. `StreamEvent` /
  `StreamId` move into the crate too.
- **Persistence**: `adoption.rs` stops calling `fathom_config_dir()`; `AdoptionStore` takes its file
  path. The `set_thread_state_root` test helper is replaced by a temp dir passed to the store.
  Depths does not use adoption at all (see R6).
- **Command classifier**: `adoption::requirement(&BackendCmdV1)` stays in Fathom.
  The crate keeps `requirement_for_scope`.
- **Kubernetes bits**: `cluster_link.rs` stays in Fathom. Its effect on preview becomes an
  optional `&dyn ClusterImpact` in `PreviewContext`; Depths passes `None`. The kind-label parse in
  `inventory.rs` becomes a post-pass in Fathom over the parsed containers. The wire fields
  (`Container.cluster`, `OperationPreview.breaks_clusters`, `AdoptionState::ImplicitViaManagedCluster`)
  keep their shape so the protocol does not break; Depths never sets them.
- **Branding**: operator-facing strings that say "Fathom" (`preview.rs:73-90`) take the app name
  from the caller.
- **Test helpers** used across modules (`ScriptedProcess`, the mock runners) move behind a
  `test-util` cargo feature of the crate, which Fathom's dev-dependency enables.

**Alternatives**: copying the modules into Depths (forbidden by FR-043); moving the whole
`fathom` backend into crates first (a much larger change with no benefit to Depths); making Depths
depend on the `fathom` crate (it pulls `kube`, `k8s-openapi` and the desktop backend into a Docker
tool).

## R4. Reusing the hosted server's sign-in, sessions, CSRF and audit

**Decision**: two library crates, neither depending on `fathom` or `kube`.

- `crates/web-auth`:
  - `session.rs` and `csrf.rs` move as they are; they depend only on std, tokio, `sha2`, `rand`,
    `secrecy`, `subtle` and `url`.
  - `oidc.rs` moves with its constructor narrowed from `HostedConfig` to `OidcConfig` (issuer,
    client id, secret, redirect URL, scopes) plus `ClaimsConfig`. `groups_required` becomes a plain
    flag; the server computes it from `AccessModel` as it does today.
  - New: a `IdentityProvider` port with two adapters, `oidc` (the moved client) and `github`
    (R5), so the HTTP layer does not care which one signed a person in.
  - New, lifted from `server/src/http.rs`: `LoginStore`, cookie helpers with a configurable cookie
    name (`__Host-fathom_session` stays Fathom's; Depths uses `__Host-depths_session`), a
    `SessionUser` axum extractor, and a CSRF layer wrapping `check_mutation`.
  - `refresh.rs` stays in the server: it swaps the Kubernetes bearer token. Only `next_delay`
    moves.
  - The server keeps its `Identity` type as a re-export, so its `Identity` and
    `fathom::backend::access::Identity` are not unified here (that is outside this feature).
- `crates/audit-log`:
  - The append-only JSONL writer from `server/src/audit.rs`: one `O_APPEND` write per entry behind
    a mutex, 0700 directory and 0600 file, a tracing mirror, and a reader. It is generic over the
    entry type (`E: Serialize + DeserializeOwned`).
  - Retention becomes an option. Fathom keeps `10_000`; Depths defaults to keeping everything and
    rotating by size into numbered files, so no entry is ever rewritten (FR-036).
  - The server keeps its Kubernetes-shaped `AuditLogEntry` and its `AuditSink` impl, now as a thin
    wrapper.

**Alternatives**: `tower-sessions` / `axum-login` (a second session model next to the one Fathom's
server already tests, so rejected); a database for sessions (v1 sessions are in memory, like
Fathom's hosted server, so a restart signs everyone out; that is acceptable for a single host and
recorded in Assumptions).

## R5. GitHub sign-in

**Decision**: a GitHub OAuth App adapter in `web-auth`.

- Authorize at `https://github.com/login/oauth/authorize` with `state` and PKCE S256; exchange at
  `https://github.com/login/oauth/access_token` (the code expires in 10 minutes).
- Scopes are `read:user`, `user:email` and `read:org`. Teams come from `GET /user/teams`, and each
  maps to the group string `github:<org>/<team-slug>`. Orgs come from `GET /user/orgs` and map to
  `github:<org>`.
- Team membership is re-read at sign-in and at every session refresh (default every 15 min), and
  never cached beyond the session. Classic OAuth tokens do not expire, so the refresh is the only
  point at which revoked membership takes effect (FR-033).
- The guide warns that new orgs restrict OAuth Apps by default. Until an owner approves the app,
  that org's teams are invisible, and the person signs in with no access. The no-access page says
  this when the provider is GitHub.
- A GitHub App adapter (fine-grained, not subject to OAuth restrictions) is a later option and is
  out of scope for v1.

## R6. Authorisation model

**Decision**: roles come from a declarative file, reloaded on change.

- `roles.toml` at `DEPTHS_ROLES_FILE` (default `/etc/depths/roles.toml`) holds grants of the form
  `group → role → stacks`. Role is `viewer`, `operator` or `admin`. `stacks` is `"*"` or a list of
  names or simple globs.
- A person's effective role on a stack is the highest grant matching any of their groups.
- The file is watched and re-read; a file that fails to parse keeps the previous mapping and shows
  an admin banner with the error (the constitution's "invalid config falls back without deleting the
  user's file").
- The UI shows the mapping read-only. Editing it in the UI is out of scope for v1: it would need its
  own bootstrap admin and its own audit trail, and a file keeps it reviewable.
- Placing a stack in the stacks dir is the declaration that Depths may manage it, so 048's adoption
  gate is not used. Foreign projects are read-only for every role.

## R7. Protocol between the Depths web app and server

**Decision**: a versioned command/event protocol over one WebSocket, defined in Rust in
`crates/depths-protocol` with `ts-rs` derives. It is the same shape as Fathom's (constitution
Principle II). Plain HTTP is used only for the sign-in redirect flow, health, the audit export
download, and static files.

The global preference for GraphQL was weighed and set aside, for three reasons:

- The repository convention is the Rust-defined protocol with generated types. Repository
  conventions outrank generic preferences.
- Most of Depths is streams (logs, shell, stats, operation output, inventory changes), which fit a
  command/event socket and fit GraphQL subscriptions badly.
- The container wire types already derive `ts-rs` and can be re-exported as they are.

WebSocket hijack protection: the upgrade checks `Origin` against the public URL and requires the
session cookie. Every command is authorised server-side (FR-032).

## R8. Socket proxy

**Decision**: the example deployment uses `linuxserver/socket-proxy` (3.4.7-r0-ls103, a drop-in
for Tecnativa that is maintained more actively) with `POST=1 CONTAINERS=1 IMAGES=1 NETWORKS=1
VOLUMES=1 EXEC=1 INFO=1 EVENTS=1 PING=1 VERSION=1 ALLOW_LOGS=1`, and Depths with
`DOCKER_HOST=tcp://socket-proxy:2375`.

- `compose down` needs DELETE, which only `POST=1` allows. The guide states plainly that
  `CONTAINERS=1` with `POST=1` is still root-equivalent (Tecnativa #182). The proxy narrows the
  surface; it does not remove that.
- `BUILD`, `SESSION` and `GRPC` are documented as needed only for stacks that build images.
- Both proxies cut idle connections after 10 minutes, so log, shell and event streams reconnect
  with backoff and the tab says it reconnected (FR-026/027). An end-to-end test runs an
  interactive exec through the proxy (exec hijack has broken in proxy releases before, Tecnativa
  #132).
- Through the proxy Depths needs no access to the socket file, so it runs as a non-root user with
  no `docker` group (FR-042). With a direct socket mount, the guide shows `group_add` with the
  socket's GID.

## R9. Watching for change while staying idle

**Decision**: watch only while someone is connected.

- A `docker events --format '{{json .}}'` stream (through the 048 `StreamRunner`) and a
  filesystem watcher (`notify` crate) on the stacks dir both run only while at least one session
  is connected.
- Changes are debounced (250 ms) into a re-inventory.
- With no sessions, Depths runs nothing, so idle CPU is effectively zero (SC-008).
- `notify` is new; inotify-based polling of mtimes was the alternative and wakes up regularly for
  nothing.

## R10. Frontend sharing

**Decision**: a root pnpm workspace with three shared packages. Fathom's store-bound container
feature stays in `ui/`.

- `packages/theme` (`@companion/theme`): `tokens.css`, `effects.css`, `color.ts`, `fonts.ts`,
  `BrandMark`, `CircuitTrace`, `applyTheme`, and the `cn` re-export. `applyTheme` takes a structural
  `ThemePrefs` type declared in the package. Fathom's generated `ThemePrefs` has the same shape, so
  it stays assignable.
- `packages/ui` (`@companion/ui`): the 22 shadcn primitives, with `@/` imports rewritten to
  relative ones. `sonner.tsx` takes the theme mode as a prop instead of reading Fathom's store.
- `packages/terminal` (`@companion/terminal`):
  - `TerminalSurface`, made store-free: it takes a `TerminalTransport` plus a `subscribe(onChunk)`
    source instead of reading `useStore` and `RingBuffer`.
  - `RingBuffer`.
  - A **virtualised** `LogView`, built on `@tanstack/react-virtual`, which Fathom already uses.
    Fathom's `LogScroller` is not virtualised, and SC-005 needs one.
  - `PaneStates`.
  - `YamlDiffView`.
  - Fathom's `TerminalView` and `LogScroller` become thin wrappers that adapt Fathom's store onto
    these, so Fathom's behaviour and tests are unchanged.
- Fathom's `ui/src/features/containers/*` does not move. Every component reads Fathom's zustand
  store, so splitting them would be a rewrite with nothing gained for Fathom. Depths writes its own
  stack-centred components on the shared primitives.
- Package scope: `@companion/*`, private, never published. It is a placeholder for the suite's name
  and can be renamed in one search-and-replace.
- **Single root lockfile.** `ui/pnpm-lock.yaml` becomes `/pnpm-lock.yaml`. About 15 places
  hard-code the ui path and are updated in one commit, verified by `scripts/ci.sh`:
  - `pipeline/src/pipeline.rs:129-228`
  - `scripts/ci.sh:40-107`
  - `scripts/build-windows.sh`
  - `scripts/third-party-licenses.sh`
  - `.github/workflows/release.yml:91-110`
  - `deploy/Dockerfile:2-9`
  - `Makefile`
  - `Makefile.toml:112`
  - `k8s/cluster.sh:115`
  
  `ui/` stays where it is rather than moving to `apps/fathom`. That move would touch every Tauri,
  Playwright and release path for no Depths benefit, so it is left as a later, separate change.
- **YAML editor**: CodeMirror 6 (`@codemirror/lang-yaml`, `@codemirror/lint`, `@codemirror/merge`
  for the diff). Fathom uses a plain `Textarea` and has no highlighter, and FR-017 needs
  highlighting with line-anchored errors. It is loaded with `React.lazy` so the stack list does not
  pay for it. Monaco was rejected for its size (several MB, web workers) on a page that also hosts
  terminals.

## R11. Packaging

**Decision**: `apps/depths/Dockerfile`, multi-stage.

1. `chef` (`rust:1.99.0-trixie` + cargo-chef): planner → cook → build `depths`.
2. `web` (Node 24 + corepack pnpm): `pnpm --filter @companion/depths-web build`.
3. `cli`: copy the `docker` and `docker compose` plugin binaries from the official `docker:29-cli`
   image.
4. Runtime `debian:trixie-slim`, holding the binary, `/usr/share/depths/web` (passed as
   `--static-dir` / `STATIC_DIR`, never embedded), the docker CLI and compose plugin, `tini`, and
   user `depths` (uid 10001).

The example `apps/depths/compose.yaml` sets `restart: ${RESTART_POLICY:-no}` on both services.

## R12. Telemetry and flags

**Decision**:

- `tracing` (as the server already uses) with an optional OpenTelemetry OTLP exporter
  (`tracing-opentelemetry`, `opentelemetry-otlp`). It is enabled only when
  `OTEL_EXPORTER_OTLP_ENDPOINT` is set.
- Runtime feature flags as environment variables, all read at start-up and shown on the admin
  page:
  - `DEPTHS_FEATURE_SHELL` (default on): turns interactive shells off entirely, for
    least-privilege installs.
  - `DEPTHS_FEATURE_EDIT` (default on): makes every stack read-only in the UI and API.

## R13. Discord sign-in

**Decision**: a Discord OAuth2 adapter in `web-auth`, beside `github`, behind the same
`IdentityProvider` port. Discord is not used as OIDC.

- Authorize at `https://discord.com/oauth2/authorize`, exchange and refresh at
  `https://discord.com/api/oauth2/token`, revoke at `https://discord.com/api/oauth2/token/revoke`.
  Always `state`; PKCE S256 is sent too. Third-party clients use it against Discord, but Discord's
  docs don't list the parameters, so the adapter's test against a real application (T088) confirms
  it, and the adapter must not depend on PKCE for its security (`state` and the confidential client
  secret carry that).
- The redirect URI is matched exactly, so it is `<public_url>/auth/callback/discord` and nothing
  else; the guide says to register that string.
- Scopes `identify email guilds guilds.members.read`. Identity from `GET /users/@me` (subject = user
  id, username, verified email when present).
- Groups:
  - `discord:<guild_id>` for every guild in `GET /users/@me/guilds`;
  - `discord:<guild_id>/<role_id>` from `GET /users/@me/guilds/{guild_id}/member`, called **only** for
    guild ids that some grant in `roles.toml` names, because that endpoint is tightly rate limited;
  - `discord:user/<user_id>`, so a one-person homelab can grant a single account without a guild.
  Guild and role ids are numeric snowflakes, so `user` cannot collide with a guild id.
- Re-read at sign-in and on every session refresh (FR-033), using the refresh token when the
  access token has expired. A 401 from Discord ends the session (T072's fail-closed rule); a 429
  is "unreachable" (keep last known groups, back off for `retry_after`).
- Alternatives: Discord's OIDC support (rejected: groups would still need the REST calls above, so
  it adds a second code path for no gain); a bot token reading guild members (rejected: a second
  credential with far more reach than sign-in needs).

## R14. Settings a Dockge host already has

**Decision**: accept Dockge-style and generic names as aliases, below Depths' own names.

| Alias | Means | Precedence |
|---|---|---|
| `DOCKGE_STACKS_DIR` | `DEPTHS_STACKS_DIR` | flag > `DEPTHS_*` > alias > default |
| `ORIGIN_URL` | `DEPTHS_PUBLIC_URL` | same |
| `OAUTH_PROVIDER` = `oidc` \| `github` \| `discord` | which provider `OAUTH_CLIENT_ID`/`OAUTH_CLIENT_SECRET` (or `_FILE`) belong to | the provider's own `DEPTHS_<P>_CLIENT_*` win |

- `ORIGIN_URL` without a scheme (`depth.boop.ninja`) is read as `https://`.
- `OAUTH_CLIENT_*` without `OAUTH_PROVIDER` is a start-up error naming `OAUTH_PROVIDER`, never a
  guess. With `OAUTH_PROVIDER=oidc`, `DEPTHS_OIDC_ISSUER` is still required.
- Start-up logs which name each setting came from (never the value of a secret), so an alias that
  silently wins is visible.
- Default listen moves from `0.0.0.0:5001` to `0.0.0.0:8080`. Running beside Dockge (the first real
  deployment) needs a different port anyway, and a swap is one port mapping.
