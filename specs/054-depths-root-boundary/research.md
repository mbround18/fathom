# Research: Depths root boundary and passkeys (054)

## Threat model

- **In scope:**
  - A compromised Depths server process: remote code execution in Depths, or a malicious build
    of it running with its normal privileges.
  - A stolen provider account.
  - A stolen session cookie.
- **Goals:**
  - A compromised Depths cannot get root on the host through the container runtime (US1–US2).
  - A compromised Depths cannot widen the volume helper (US3).
  - A stolen account or cookie cannot act as an administrator, or take a risky action, without the
    passkey (US4–US5).
- **Out of scope:** passkeys do not protect against a compromised Depths server. The server checks
  them, so a server that has been taken over can skip the check. US4–US5 defend the sessions; the
  guard defends the host.
- **Residual risk** (written in the guide, per FR-023). A compromised Depths can still:
  - run, stop and delete containers that are not escape-level;
  - pull images;
  - read and write the stacks directory;
  - mount any named volume, including other projects' volumes on the same daemon;
  - use every exception the operator granted, by labelling a container as the stack and service
    that hold the exception.

## R1. Where the guard sits

- **Decision:** add a new service, `guard`, which runs the binary `depths-guard` from its own
  `FROM scratch` image. It sits between Depths and the existing socket proxy:

  ```
  depths ──(net depths-guard, internal)── guard ──(net docker-api, internal)── socket-proxy ── docker.sock
  ```

  - Depths leaves the `docker-api` network, so `guard` is its only route to the runtime.
  - The guard has no runtime socket. It runs as uid 65532, has no capabilities, a read-only root,
    no-new-privileges, and pids and memory limits.
  - The socket proxy keeps its endpoint filter, as a second layer.
- **Why:**
  - The guard parses untrusted JSON. Giving it no socket and no privileges means a bug in the guard
    is not root on its own.
  - The socket proxy is already deployed and proven, and stays as defence in depth.
  - The 051 rules already exist in Rust (`depths-core::security`), so the guard reuses them rather
    than restating them in a proxy config language.
- **Rejected:**
  - Replacing the socket proxy, so the guard holds the socket. This is one hop fewer, but a guard
    bug would then be root.
  - A Docker authorization plugin (`--authorization-plugin`). It needs daemon configuration on the
    host, which this deployment does not own. Plugins also see the request too late to rewrite it,
    and a broken plugin can wedge the daemon.
  - Open Policy Agent with a Rego policy. It adds a second rule language and a large runtime, and
    the rules would drift from `depths-core`.

## R2. Shared rules

- **Decision:** add a new agnostic crate, `crates/depths-guard-core`, with no I/O. It does three
  things:
  - classifies a request (method and path, with the `/v1.NN` prefix removed) into an `Endpoint`;
  - parses the request bodies the guard judges into typed structs;
  - returns a `Verdict`.
- It calls into `depths-core::security` (`normalise_host_path`, `classify_host_path`,
  `normalise_cap`, `unconfined_opt`), so the guard's answers and the 051 lint's answers come from
  one place.
- The binary in `apps/depths/guard` is the adapter. It handles HTTP proxying, upgraded connections,
  inspecting through the upstream, reading the filesystem for symlink checks, and the log.
- **Why:** this follows the repo's agnostic core and adapters pattern. It also lets the full rule
  table be tested deterministically, without Docker.

## R3. What is refused (FR-003, FR-005)

### Request kinds

The guard denies by default. It allows only the request kinds that Depths, `docker compose` and
the volume helper send:

- `_ping`, `version`, `info`, `events`;
- containers: list, inspect, logs, stats, top, wait, attach, resize, create, start, stop, restart,
  kill, pause, unpause, remove, update, and exec create, start, resize and inspect;
- images: list, inspect, create (pull), remove, and `distribution/*/json`;
- networks: list, inspect, create, remove, connect and disconnect;
- volumes: list, inspect, create and remove.

The final list is recorded from real traffic. A debug-only recording mode
(`#[cfg(debug_assertions)]`) logs each request kind while the example stacks run compose `up`,
`down`, `pull`, `logs`, `exec` and the helper, and those recordings become fixtures.

### Refused by default, even when the kind is allowed

Image builds (`/build`, `/session`) are refused by default. BuildKit's session channel is opaque,
so the guard cannot judge what build rights it asks for. The operator can opt in with
`allow_build = true`; the guide says this weakens the boundary. Stacks that build fail with a
refusal that names the setting.

Everything else is refused: `archive`, `export`, `commit`, `exec` into protected containers,
plugins, swarm, secrets, configs, nodes, services, `images/load` and `images/*/push`.

### Container create and update

**Escape settings.** A create is refused if any of these are set:

- `Privileged`;
- `PidMode`, `IpcMode`, `UTSMode`, `UsernsMode` or `CgroupnsMode` set to `host`;
- `NetworkMode` set to `host`;
- `CapAdd` with `ALL`, or with anything outside Docker's default set: CHOWN, DAC_OVERRIDE,
  FSETID, FOWNER, MKNOD, NET_RAW, SETGID, SETUID, SETFCAP, SETPCAP, NET_BIND_SERVICE, SYS_CHROOT,
  KILL and AUDIT_WRITE;
- the `Capabilities` field;
- `Devices`, `DeviceRequests` or `DeviceCgroupRules`;
- a `SecurityOpt` that `unconfined_opt` flags (`seccomp=unconfined`, `apparmor=unconfined`,
  `label=disable` or `label:disable`), or `systempaths=unconfined`;
- `MaskedPaths` or `ReadonlyPaths` set to anything other than empty or absent (they unmask
  `/proc`);
- a `Runtime` other than the daemon default, unless the operator lists it.

**Host paths.** Every place a host path can appear is judged:

- `Binds` entries whose source is absolute;
- `Mounts` of type `bind`;
- `Mounts` of type `volume`, and named volumes in `Binds`, whose volume has local-driver bind
  options (`type=none`, `o=bind`, `device=/...`). This is checked by inspecting the volume through
  the upstream, and also when such a volume is created.

A host path is refused unless it is under an allowed root (R5). Writable sensitive paths are
always refused.

**Protected objects.** `VolumesFrom`, and the `container:<id>` forms of `NetworkMode`, `PidMode`
and `IpcMode`, are refused when they name a protected container (R6).

**Update.** A container update can carry `Devices`, `DeviceRequests` and `DeviceCgroupRules`, so
those are judged the same way.

**Exec create.** Refused for `Privileged: true`, for any non-empty `User` (Depths never sets one;
see `ws/streams.rs` shell argv), and for any exec into a protected container.

**Start.** A start or restart re-inspects the container through the upstream and judges its stored
config again, because bind sources are resolved when the container starts.

- So a container that predates the guard keeps running; the daemon's restart policy restarts it
  without a request.
- But an explicit start of an escape-level container through Depths is refused unless an exception
  covers it. The upgrade notes say so (spec edge case, "judges only new requests").

**Networks.** Connecting a container to a protected network, creating one with the same name, or
removing one is refused. `NetworkingConfig.EndpointsConfig` on create is judged the same way.

**Failure handling.**

- Bodies are capped at 1 MiB.
- JSON that cannot be parsed, and fields the guard does not know in `HostConfig`, are refused.
  Docker adds fields over time, so the guard fails closed on new fields and its allowlist grows
  with tests.
- The rules are written to the guard's log in its own words, with the 051 `Code` name where one
  exists.

## R4. Exceptions (FR-008, clarified Q1 = A)

- **Decision:** exceptions live only in `/etc/depths-guard/guard.toml`, mounted read-only from the
  host. The guard reads the file at start and on `SIGHUP`. A file that does not parse stops the
  guard from starting, which fails closed. See `data-model.md` for the shape.
- **Matching.** An exception matches on:
  - the Compose labels `com.docker.compose.project` and `com.docker.compose.service`;
  - optionally the image repository, which is recommended and suggested by `check`.

  It allows only the settings it lists: device paths, host roots (read-write or read-only),
  capabilities, and the network, PID or IPC namespace.
- **Never grantable:**
  - `Privileged`;
  - a runtime socket path;
  - writable sensitive host paths.

  A file that lists any of these is refused at load.
- **Read access for Depths:** the guard serves `GET /_depths_guard/v1/exceptions` on the same
  listener, read-only. Depths uses it to show a stack's exceptions (FR-009). The guard has no
  write endpoint.
- **Why:** Q1 = A, so nothing reachable from inside Depths can widen the guard.
- **Residual:** labels can be forged, so an exception means "this reach is granted to whatever runs
  as this project and service". The image match narrows that. The guide says so.

## R5. Allowed roots and symbolic links

- **Allowed roots:**
  - `stacks_dir`, which is required. It is the host path, and is identical to Depths'
    `DEPTHS_STACKS_DIR`.
  - The operator's `roots.allow` list.
  - Sensitive paths (051 `classify_host_path`) are refused even when they sit under an allowed
    root.
- **The problem:** Depths writes the stacks directory, so a compromised Depths can plant
  `stacks/x/l -> /etc` and bind `stacks/x/l`. Docker follows the link.
- **Decision:** the guard mounts each allowed root read-only at the same path. It judges a bind
  source by walking it from the root with `openat2(RESOLVE_BENEATH | RESOLVE_NO_SYMLINKS)`, and
  refuses if any component is a symbolic link, or if the source does not exist. Docker would
  create a missing source as root, so a missing source is refused too. The walk runs at create
  and again at start.
- **Residual:** a race between the guard's check and the daemon's mount. This is documented. The
  guide recommends mounting the stacks filesystem with `nosymfollow` (Linux 5.10 or later) as
  host-side hardening that closes it.
- **Rejected:** resolving paths by string alone. That is wrong for links, and is the bypass above.

## R6. Protected objects

- **Protected containers:** the guard and the socket proxy. They carry the label
  `org.fathom.depths.protected=true`, and the guard also resolves them by name at start, from
  `protected.containers`.
- **Protected networks:** `docker-api` and `depths-guard`.
- **Refused against protected containers:** stop, kill, remove, update, exec, attach, pause, and
  every namespace or volume join.
- **Refused against protected networks:** connect, create and remove.
- **Why:** without this, a compromised Depths could start a container on `docker-api` and talk to
  the socket proxy directly, or use `--volumes-from socket-proxy` to get the runtime socket.
- Stopping the guard still fails closed, because nothing reaches the runtime without it. Refusing
  it keeps the guard's log and its availability out of Depths' hands.

## R7. Guard log (FR-006)

- **Decision:** the log is JSON lines in a host folder, mounted read-write only into the guard
  (default `./guard-log`). It is also written to stderr. The host folder is outside the allowed
  roots, so no stack and no helper can mount it.
- **Rotation:** at 10 MiB, keeping 5 files.
- **Each entry:** time, request kind, project and service when known, outcome, rule or exception,
  and the offending setting. Host paths are shown as the setting they were judged on, never the
  file contents.
- **Rejected:** logging only to stdout. Removing the container would erase the log, and Depths can
  read container logs.

## R8. How refusals reach Depths (FR-007)

- The guard answers a refusal with `403` and the body
  `{"message":"depths-guard refused <rule>: <setting> [<project>/<service>]"}`.
- The Docker CLI and compose print this as
  `Error response from daemon: depths-guard refused ...`.
- `depths-runtime-cli` parses the prefix from the output of a failed op into a
  `GuardRefusal { rule, setting, service }`. Depths attaches it to `OpFinished` and to the helper's
  refusal, and writes an audit entry (`AuditAction::GuardRefused`).
- If the guard cannot be reached (connection refused), the result is `RuntimeGuardUnreachable`,
  shown as "the runtime guard is not answering".

## R9. Helper image (FR-011 to FR-014)

- **Decision:** a new Dockerfile target, `helper`. It is `FROM scratch`, containing only
  `COPY --from=builder /src/depths-volume-helper /depths-volume-helper`, with the label
  `org.fathom.depths.helper.build=<build id>`.
  - The main image no longer contains the helper. The `--entrypoint` becomes
    `/depths-volume-helper`.
- **Run flags:** these stay exactly as in 053 (contracts/helper.md): no network, `--cap-drop ALL`,
  read-only, no-new-privileges, 256 MiB, 32 pids.
  - The spec asked for "the few caps it needs". The helper runs as the owner of the object it acts
    on (053 R5), so it needs none. The spec now says so.
- **Build id:** the builder computes the helper binary's SHA-256 once and passes it to both
  binaries at compile time, as `DEPTHS_BUILD_ID` through `option_env!`.
  - `Hello` gains `build: String`, and the server refuses a helper whose `build` differs from its
    own (FR-013).
  - At start the server also resolves `DEPTHS_HELPER_IMAGE` to an image id and checks the label.
  - `DEPTHS_HELPER_IMAGE` is now required. Its compose default is
    `${DEPTHS_HELPER_IMAGE:-depths-volume-helper:dev}`. There is no fallback to the main image.
- **Tests:** `files_docker.rs` seeds volumes with the main image, which still has `/bin/sh`, and
  runs the helper from the helper image.
- **Rejected:**
  - Distroless static: it carries more files than needed.
  - Keeping the helper in the main image: the main image has a shell, curl and the Docker CLI.

## R10. Passkey library (FR-015, FR-018)

- **Decision:** `webauthn-rs` 0.5 (the latest is 0.5.x), using its `Passkey` flows, with `openssl`
  built `vendored` so the server stays a self-contained binary. Tests use
  `webauthn-authenticator-rs`'s soft passkey as a dev-dependency.
  - The relying party id is `public_url`'s host, and the origin is `public_url`'s origin; both
    already exist in config. Loopback development uses `localhost`.
  - Ceremony state is kept in memory, keyed by a random ceremony id bound to the session handle,
    with a 2-minute expiry.
- **Why:** webauthn-rs is the most reviewed Rust relying-party library (Kanidm), and it handles
  attestation and counter checks.
- **Cost:** it brings OpenSSL into the server build. `deny.toml` and the cargo-chef cache cover it.
- **Rejected:**
  - `webauthn-rp`, which is pure Rust but young.
  - A hand-rolled verifier, which is security-critical code we would own.
  - `passkey` (1Password), which is aimed at clients and authenticators.

## R11. Where verification lives (FR-016, FR-017, FR-022)

### State

- `SessionMeta` gains `verified_at: Option<DateTime<Utc>>`. It is held server-side, per session
  handle, and dropped with the session.
- Each browser has its own session, so each verifies on its own (spec edge case).
- A copied cookie carries the session, so it also carries `verified_at`. That is why risky actions
  need a fresh check (FR-017), and why the session's IP and user-agent checks from 052 still
  apply.
- SC-005 is met for cookies copied before verification, and for any risky action after the window
  closes.

### The gate

A new `StepUp` gate in `ops.rs` runs after the 052 trust gate. Both must pass (FR-022).

- **Admin and owner without `verified_at`:** their effective access becomes the restricted view,
  through the same path 052 uses for untrusted networks.
- **Risky commands (any role):** `OpenShell`; `Confirm` of a preview whose
  `CommandKind::StepUp(_)` holds; `PutUser`, `RemoveUser`, and network whitelist changes; passkey
  removal.
  - The preview is that of a save or deploy with escape findings, an acknowledgement, a file write,
    upload, rename or delete, or a restore.
  - These need `verified_at` within `step_up_window` (default 300 s, set by
    `DEPTHS_STEP_UP_WINDOW`), or an assertion carried by the command itself.

### Fitting into the confirm step (FR-019)

- A preview that will need step-up carries `step_up: Some(PasskeyRequest)`, which holds the
  challenge.
- `Confirm` carries the `assertion`. The client calls `navigator.credentials.get` when the person
  presses Confirm, so there is one prompt in the existing dialog.
- `OpenShell` has no preview. A stale shell request is answered with `StepUpRequired { request }`,
  and the client retries with `assertion`.

### Forced enrolment (FR-016, Q2 = A)

- A person without a passkey gets `PasskeyEnrolmentRequired` for any gated command: admin actions
  for admins and owners, and risky actions for everyone.
- Registering the first passkey needs no prior assertion. Registering later ones needs a fresh
  step-up.
- **Residual:** someone holding a stolen provider account could enrol first, before the real
  person enrols. The guide tells owners and admins to enrol right after upgrading. Recovery (R12)
  fixes it afterwards.

## R12. Owner recovery (FR-021)

- **Decision:** `depths passkey-recovery --user <user id>` is a subcommand of the server binary.
  The operator runs it on the host with `docker compose exec depths depths passkey-recovery ...`,
  which needs host runtime access and not Depths.
  - It writes the SHA-256 of a new 10-character code to `<data_dir>/passkey-recovery.json`, with a
    15-minute expiry and single use, and prints the code once.
- In the UI, the signed-in person enters the code on the account page. That removes all of their
  passkeys and marks the session as able to enrol. Both steps are audited.
- **Why:** the code is only reachable from the host. A provider account on its own cannot recover.

## R13. Feature flags

- The guard, the helper image and passkeys are security boundaries, so there is no runtime off
  switch in release builds.
- `step_up_window` is set at runtime.
- `allow_build` and the exceptions are runtime settings in `guard.toml`.
- Only the guard's request recording is a development-only compile-time flag.

## R14. Web client

- **Decision:** use `navigator.credentials` directly, with a ~40-line base64url codec, in
  `apps/depths/web/src/features/passkeys/`. No new dependency.
- **Tests:** Playwright uses Chromium's virtual authenticator through CDP
  (`WebAuthn.addVirtualAuthenticator`) against the fake backend. The fake backend accepts any
  well-formed assertion; real verification is tested in Rust.
- **Rejected:** `@simplewebauthn/browser`. It is a dependency for what amounts to two calls and a
  codec.
