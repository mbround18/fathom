# Research: Depths container hardening

## R1 Where the rating comes from
- **Decision**: `docker inspect` fields (`Config.User`, `HostConfig.{Privileged,CapAdd,PidMode,IpcMode,UsernsMode,NetworkMode,SecurityOpt,Devices}`, `Mounts`) plus `docker info --format '{{.SecurityOptions}}'` for daemon mode (`name=rootless`, `name=userns`).
- **Rationale**: the runtime truth, independent of how the container was created.
- **Alternatives**: compose file only (misses `docker run` containers and drifts) — used for lint instead.

## R2 Root vs rootless
- **Decision**: `root` = uid 0 (empty user, `0`, `root`, `0:*`) on a rootful daemon. On a rootless or userns-remap daemon uid 0 is not host root, so it is `ok` with an informational reason. Undetectable daemon mode counts as rootful (fail closed).

## R3 Escape set
- **Decision**: privileged; runtime sockets (docker, containerd, podman, cri) always; writable binds of `/`, `/etc`, `/proc`, `/sys`, `/dev`, `/boot`, `/root`, `/var/lib/docker`, `/run`; read-only `/` or `/etc` is `root`-level; host pid/ipc/userns; `SYS_ADMIN`, `SYS_PTRACE` (with host pid), `SYS_MODULE`, `DAC_READ_SEARCH`, `NET_ADMIN`, `BPF`, `SYS_RAWIO`, `ALL`; seccomp/apparmor/selinux unconfined; device passthrough.

## R4 Decision timing
- **Decision**: fresh single-container inspect inside the shell-open path, after `permits`; any error ⇒ refuse with "could not verify". Cached ratings are display only.

## R5 Exec hardening
- **Decision**: `docker exec --interactive --tty [--env TERM=xterm-256color] <id> <shell>`; never `--privileged`, never `--user`/`--workdir` from the client; shell path validated against an allow-list pattern; container arg validated as an id.

## R6 Ending shells on recreate
- **Decision**: shells are keyed by container id; a `docker events` destroy/rename for that id or a recreate observed by the inventory closes the shell with a reason.

## R7 Composition with 052
- **Decision**: `shell_decision(policy, level, is_admin) -> Result<(), Refusal>` stays pure; 052's network gate runs before it in the same path; both must allow.
