# Research: Depths volume explorer

## R1 How Depths reaches a volume
- **Decision**: a helper container per operation or session, created with `docker run -i` through
  the socket proxy that Depths already uses.
  - The helper is made from Depths' own image, with the `depths-volume-helper` entrypoint.
  - It mounts exactly one source at `/volume`: the volume's name for a named volume, or the
    resolved host path for a bind mount.
  - Its run flags are fixed in the adapter (contracts/helper.md): `--network none`, `--read-only`,
    `--cap-drop ALL`, `--security-opt no-new-privileges`, `--user <uid>:<gid>`, `--pull never`,
    `--rm`, `--log-driver none`, plus memory, pids and CPU limits and the `depths.helper=1` label.
- **Rationale**:
  - Depths can already create containers (`CONTAINERS=1`, `POST=1`), so this adds no capability
    (FR-020).
  - The kernel enforces the helper's boundaries, not Depths' code: it sees one mount, and has no
    network and no capabilities.
  - It works the same for named volumes, which Depths cannot see without `/var/lib/docker`, and
    for bind mounts.
- **Alternatives**:
  - Mounting the stacks root writable plus `/var/lib/docker/volumes` into Depths: standing access
    to every stack, and it breaks the lockdown.
  - `docker cp` / the archive API: it runs as root inside the daemon, ignores file owners
    (FR-021), cannot do atomic in-place saves, and needs `ARCHIVE` in the proxy.
  - `docker exec` into the service's own container: it needs `EXEC`, the image may have no tools,
    and it does not work on stopped stacks.

## R2 Image for the helper
- **Decision**: `DEPTHS_HELPER_IMAGE` sets the image. By default, at start, Depths resolves its
  own image ID by inspecting its own container, found through `HOSTNAME` (the container ID).
  - If neither works, the feature is off, with the reason given to admins in `/api/me`.
  - The helper is always run by image ID with `--pull never`, so a tag change cannot swap it.
- **Rationale**: the image is always present on the host and versioned with the server, so the
  frame protocol version always matches.

## R3 Path containment
- **Decision**:
  - Every path is first validated as a `RelPath` in `depths-volumes`: relative, no NUL, no empty
    or `.`/`..` components after normalising, at most 4,096 bytes, and each component at most 255
    bytes.
  - The helper then opens it with `openat2(dirfd(/volume), path, RESOLVE_BENEATH |
    RESOLVE_NO_MAGICLINKS)`.
  - The final component is opened with `O_NOFOLLOW` against its parent `O_PATH` fd and checked
    with `statx`.
  - `EXDEV`/`ELOOP` from a link out of the root is reported as `outside_mount` and audited.
- **Absolute links** are never followed, even when they would land inside, because their meaning
  depends on the service container's view. They are listed with their target.
- **Check-to-use races**: every operation works on fds opened beneath the root, so swapping a link
  between a list and a write cannot redirect the write. The escape suite (SC-003) swaps links
  concurrently.
- **Old kernels**: on start, the helper probes `openat2`. `ENOSYS` makes the feature off for that
  server, with the reason "kernel older than 5.6 or seccomp blocks openat2". There is no fallback
  path resolver.

## R4 Helper lifetime and pooling
- **Decision**: a warm helper per (mount, uid, mode), kept for 90 s idle, with at most 8 per
  server. The least recently used one is closed first. Transfers each get a one-shot helper, so a
  long download never blocks browsing.
- On start, Depths removes leftover containers labelled `depths.helper=1` (from a crash).
- **Rationale**: a cold `docker run` takes about 0.3–0.8 s, which is fine once but not on every
  folder click.

## R5 Which uid the helper runs as (FR-021)
- **Decision**: the helper runs as the owner of the object it acts on:
  - reads and saves of a file: the file's uid:gid;
  - listing a folder: the folder's;
  - create, rename and delete: the parent folder's.
- The owner comes from the listing that led to the operation, and the helper re-checks it with
  `statx`. If the owner changed, the reply is `owner_changed` and the UI reloads.
- **Rename across owners**: a rename whose source and destination folders have different owners
  is refused `mixed_owners`; no single uid could do it without privileges.
- **Recursive delete** runs as the folder's owner. If it reaches an entry that uid cannot remove,
  it stops and replies `partial` with what it removed and the system error, shown verbatim.
- **Root-owned objects**: the helper runs as uid 0 with no capabilities, so it has no DAC override
  and gets only the owner's mode bits.
- **Mount root**: a stat of the mount root runs as `65534:65534`, since stat on `/volume` needs no
  permissions.
- **Rootless and userns-remapped daemons**: uids are in the daemon's namespace, which is the same
  view the service has.

## R6 Atomic save that keeps metadata (FR-007, SC-004)
- **Decision**:
  1. Open the parent beneath the root.
  2. Create `.depths-save-<random>` with `O_EXCL`, write it, `fchmod` it to the old mode, and keep
     its uid and gid (the helper runs as them).
  3. `fsync` it, `renameat` it over the target, then `fsync` the parent.
  - If any step fails, the temp file is unlinked.
  - If the parent is not writable by the owner, the save is refused with the system error and a
    note. It is never an in-place truncate.
- **Known loss**: extended attributes and ACLs are not copied. They are rare on game and home
  server volumes, and the guide says so.
- **Hard links**: a file with `nlink > 1` is read-only (Edge Cases), which also avoids breaking
  the link.

## R7 Change detection (FR-008)
- **Decision**: the fingerprint is the SHA-256 of the content plus `(size, mtime_ns, ino)`.
  `ReadFile` returns it, and a save carries it as `base`. The helper recomputes it on the open fd
  immediately before the rename; a mismatch replies `changed_on_disk`, with the current
  fingerprint.
- **Rationale**: two saves seconds apart can share an mtime on coarse filesystems, but the hash
  catches it. The window between recompute and rename is microseconds, and the guide documents it.

## R8 Text handling (FR-006, FR-007)
- **Decision**:
  - Content is text when it is valid UTF-8, has no NUL in the first 8 KiB, and is within the
    editor limit. Anything else is binary and offers download only.
  - A BOM is stripped and flagged (`bom: true`).
  - Line endings are detected as `lf`, `crlf` or `mixed`. The editor works in LF, and the server
    re-applies CRLF on save.
  - `mixed` opens with a notice that the save will use the most common ending.
  - The trailing newline is kept as is.
- The re-application lives in `depths-volumes::text`, with property tests: decode then encode is
  the identity for every non-mixed input.

## R9 History store (FR-015, FR-016)
- **Layout**:
  - `<data_dir>/file-history/blobs/<sha256>`, written with temp + rename.
  - `<data_dir>/file-history/files/<sha256(stack \0 mount \0 path)>.json`: the list of versions
    (time, author, size, blob, reason), written atomically.
- **Capture**: the previous content comes from the helper's write reply (`prior` frames) when it
  is at most 10 MiB, so it is the exact bytes that were replaced.
- **Pruning**: a pure policy in `depths-volumes::history` decides what to drop by per-file count,
  then age, then total size, oldest first. It runs after each write and once at start. A blob is
  removed when no `files/*.json` refers to it.
- **Rationale**: content addressing removes duplicates, such as a restore of a kept version, and
  each JSON stays small.

## R10 Mount discovery (FR-001)
- **Decision**:
  - From the inspected containers of the stack (the `Mounts` Depths already reads for 051), with
    `Name` added to `security::Mount`.
  - When a service has no container, from `docker compose config --format json` of the stack (an
    argv like the existing `config --quiet` steps), for volumes and binds. A named volume that
    does not exist yet is listed as `not_created`.
  - Only `ready` mounts get a helper. `docker run --mount type=volume` creates a missing volume,
    so the gate refuses `not_created`/`missing`, and the adapter needs proof (`docker volume
    inspect`) that the volume exists before it builds the argv.
- **Mount id**: stable and opaque, `sha256(kind \0 source)` shortened to 16 hex digits. It is
  computed by the server, so the URL is stable and the client never supplies a source.
- **Browsability**:
  - `tmpfs` and `npipe` are never browsable.
  - A bind is browsable only if `bind_reason()` rates it `info` or nothing. Anything rated root or
    escape (`/`, `/etc`, `/proc`, the runtime socket and so on) is refused with that reason.
  - A bind whose source is a socket or a single file is listed but not browsable ("single-file
    bind": it can be opened as a file, in a later version).
- **Writable**: a mount is writable when at least one service mounts it RW.

## R11 Transfers
- **Download**: `GET /api/v1/stacks/{stack}/files/download?mount=&path=`.
  - A file streams with `Content-Disposition`.
  - A folder streams as a `tar` that the helper writes to stdout, with no compression (most game
    saves are already compressed, and it is cheap on CPU).
  - The size is checked first with a bounded walk (`FolderSize`), and stops at the limit.
  - The response body streams the helper's stdout frames with backpressure, and is never
    buffered.
- **Upload**: `POST /api/v1/stacks/{stack}/files/upload?mount=&path=&name=&replace=<fingerprint|none>`.
  - It sits behind `CsrfLayer`, with its own body limit (`DEPTHS_FILES_UPLOAD_MAX`, default
    1 GiB) instead of the global one.
  - The body streams into the helper, which writes a temp file and renames it (R6).
  - Every upload goes through `PreviewFileOp{Upload}` → `Confirm` first (constitution I). The
    preview names each file and, for a name that exists, the fingerprint of the file it would
    replace (one `Stat` + hash; listings carry no fingerprints). The POST carries the preview id,
    and a mismatch at write time gets 409 `changed_on_disk`.
- **Rationale**: the existing WS has a message cap, and binary data over WS would need its own
  chunking. HTTP streaming is native in Axum and gives the browser a progress bar.

## R12 Gates (FR-017, FR-018, FR-022)
- **Decision**: one `files_gate(session, ip, stack, op_kind)` in `files.rs`, used by WS dispatch
  and both HTTP routes. In order:
  1. **Feature mode**: off → `feature_off`; read-only → only list, read and download.
  2. **Role**: edit permission on the stack (`role::permits`).
  3. **052 trust**: untrusted → `ListMounts`/`ListDir` only, everything else `untrusted_network`.
     The new kinds go into `restricted_ok`.
  4. **Mount**: browsable, and writable for writes.
- Every refusal is audited, including the ones the helper returns (`outside_mount`,
  `write_blocked`, `owner_changed`, `changed_on_disk`, `os_error`, `partial`): the server maps each
  to a `FileRefused` (or `Denied` for role, trust and feature off) and one `files.refused` entry. `/api/me` carries `features.files` (`off`, `read_only` or `full`).
  The Files tab shows only when that is not `off` and the person may edit the stack, the same
  per-stack check the Edit button uses. Viewers never see it.

## R13 Editor
- **Decision**:
  - Reuse `features/editor/CodeEditor.tsx` with new `language` and `readOnly` props. Languages
    load lazily (`lang.ts` in features/files maps names and content sniffing to a loader).
  - The parse checks run in the browser through the existing `@codemirror/lint`, for JSON (native)
    and YAML (the existing `yaml` package), plus TOML (`smol-toml`), XML (`DOMParser`) and INI (a
    small hand-written checker). They only warn (FR-009).
- **New packages**:
  - `@codemirror/lang-json`, `@codemirror/lang-xml` and `@codemirror/legacy-modes` (MIT): stable
    and maintained by CodeMirror.
  - `smol-toml` (BSD-3): stable.
  - Their licences are covered by the existing allow-list.
- **Unsaved guard**: reuse `useUnsavedGuard.ts`.

## R14 Restart after save (FR-010)
- **Decision**: after a confirmed save, if a running container mounts the mount, the result event
  carries `running_services`. The UI then offers "Save and restart <service>" as two steps: the
  save `Confirm`, then the ordinary `PreviewOp{restart, service}` → `Confirm`, with no new
  restart path. The option shows only if `permits(restart)`.

## R15 Observability
- **Decision**: OpenTelemetry spans `files.op` (kind, stack, mount id, outcome, bytes, helper cold
  or warm) and a histogram `depths.files.helper_start_ms`. Paths are not recorded as span
  attributes, since they can carry names people consider private.
