# Data model: Depths root boundary and passkeys (054)

## Guard (crate `depths-guard-core`, read by `depths-guard`)

### GuardConfig (`/etc/depths-guard/guard.toml`, read-only, host-owned)

```toml
stacks_dir = "/opt/stacks"               # required; same host path as DEPTHS_STACKS_DIR
upstream   = "tcp://socket-proxy:2375"   # default
listen     = "0.0.0.0:2375"              # default
allow_build = false                      # R3; true weakens the boundary
runtimes   = []                          # extra OCI runtimes allowed besides the default

[roots]
allow = ["/srv/games"]                   # extra host roots; read-write binds allowed beneath

[protected]
containers = ["depths-guard", "depths-socket-proxy"]
networks   = ["depths_docker-api", "depths_depths-guard"]

[[exception]]
project = "media"                        # com.docker.compose.project
service = "app"                          # com.docker.compose.service
image   = "lscr.io/linuxserver/jellyfin" # optional repository match (no tag/digest)
devices = ["/dev/dri"]
binds   = [{ path = "/mnt/library", read_only = true }]
caps    = ["NET_ADMIN"]
namespaces = ["network"]                 # network | pid | ipc
note    = "GPU transcoding"              # shown in Depths
granted = "2026-10-25"                   # shown in Depths (FR-009 "since when")
```

Validation at load (any failure: the guard does not start):

- `stacks_dir` and every root: absolute, normalised (`normalise_host_path`), not sensitive per
  `classify_host_path`, and existing.
- An exception has a non-empty `project` and `service` and at least one grant.
  - `devices`: under `/dev/`. Never a runtime socket.
  - `binds`: absolute paths. A writable bind must not be sensitive or a runtime socket.
  - `caps`: normalised (`normalise_cap`). `ALL` is never allowed.
  - `namespaces`: one of the three listed.
- There is no `privileged` key. An unknown key is a load error, so `privileged = true` cannot be
  smuggled in.
- No two exceptions may have the same (project, service, image).

### Endpoint

A request's kind is `Endpoint::{Ping, Version, Info, Events, ContainerList, ContainerInspect,
ContainerCreate, ContainerStart, ContainerUpdate, ContainerAction(Stop|Restart|Kill|Pause|Unpause|Wait|Resize|Remove),
ContainerLogs, ContainerStats, ContainerTop, ContainerAttach, ExecCreate, ExecStart, ExecInspect,
ExecResize, ImageList, ImageInspect, ImagePull, ImageRemove, DistributionInspect, NetworkList,
NetworkInspect, NetworkCreate, NetworkRemove, NetworkConnect, NetworkDisconnect, VolumeList,
VolumeInspect, VolumeCreate, VolumeRemove, Build, GuardExceptions, GuardHealth, Unknown}`.

- `classify(method, path)` strips the `/v1.NN` prefix.
- `Unknown` is always refused. So is `Build` unless `allow_build` is set.

### Verdict

- `Allow { exception: Option<ExceptionRef> }`.
- `Refuse { rule: Rule, setting: String }`.
- `NeedsUpstream(Lookup)`: the guard must first inspect a container or volume, or walk a host
  path, and then judge again with the facts it found. It never allows while a fact is missing.

### Rule

Each rule has a stable `snake_case` name. It is the 051 `Code` name where one exists:

- `privileged`, `host_pid`, `host_ipc`, `host_uts`, `host_userns`, `host_cgroupns`,
  `host_network`;
- `added_capability`, `capabilities_field`, `devices`, `unconfined`, `unmasked_paths`,
  `runtime`;
- `host_path_outside_roots`, `sensitive_path`, `runtime_socket`, `symlinked_path`,
  `missing_bind_source`;
- `volume_bind_options`;
- `protected_container`, `protected_network`;
- `exec_privileged`, `exec_user`;
- `build_disabled`, `unknown_endpoint`, `unknown_field`, `bad_body`, `body_too_large`.

### GuardLogEntry (JSON line, guard-owned folder)

`{ time, endpoint, project?, service?, container?, outcome: "allowed"|"refused"|"exception",
rule?, setting?, exception? }`. Requests that are allowed and use no exception are not logged.

## Server

### Passkey (`<data_dir>/passkeys.json`)

```json
{ "version": 1,
  "users": { "discord:123": [
    { "id": "<credential id b64url>", "name": "YubiKey 5", "created": "2026-10-25T10:00:00Z",
      "last_used": "2026-10-25T11:00:00Z", "credential": { /* webauthn-rs Passkey */ } } ] } }
```

- Writes are atomic: a temp file, fsync, then rename, as in `users.rs`.
- A file that does not load is kept and moved aside. Depths starts with no passkeys and every
  admin is asked to enrol, which is audited (constitution VI: the user's file is never deleted).
- `name` is 1–64 characters, trimmed, and unique per user.
- A person must keep at least one passkey, unless they are recovering (R12).

### Ceremony (memory only)

`{ id, session_handle, kind: Register|Verify|StepUp{preview_id?}, state, expires (2 min) }`.

- A ceremony is single-use, and bound to its session and its kind.

### SessionMeta (052) gains

`verified_at: Option<DateTime<Utc>>`, which is set on a successful Verify or StepUp ceremony.

### RecoveryCode (`<data_dir>/passkey-recovery.json`)

`{ user, code_sha256, expires, used: bool }`.

- There is at most one code per user, and a new one replaces the old.
- A code expires after 15 minutes and works once.

### Audit actions (append-only, 050)

The new actions are:

- `GuardRefused { rule, setting, service? }`;
- `PasskeyRegistered { name }`, `PasskeyRemoved { name }`;
- `PasskeyVerified`, `PasskeyFailed { reason }`;
- `StepUpRequired { action }`;
- `PasskeyRecoveryIssued` (written by the CLI), `PasskeyRecovered`.

Audit entries never hold credentials, challenges or codes.

### GuardException (read-only view)

Depths fetches exceptions from the guard (`GET /_depths_guard/v1/exceptions`) and shows them on
the stack page: `{ project, service, image?, grants[], note?, granted? }`.
