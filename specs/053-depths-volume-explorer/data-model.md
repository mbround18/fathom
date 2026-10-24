# Data model: Depths volume explorer

The types live in `depths-volumes`, and those that cross to the UI are re-exported from
`depths-protocol` with ts-rs bindings.

## Mount (server → UI)

| Field | Type | Rules |
|---|---|---|
| `id` | `MountId` (16 hex) | `sha256(kind \0 source)[..8]`, computed by the server; the only way the client names a mount |
| `kind` | `volume` \| `bind` \| `tmpfs` \| `other` | from inspect `Type`; empty is read as `bind` (051 rule) |
| `source` | string | volume name, or host path for a bind; shown to editors only |
| `services` | `[{ service, destination, read_only }]` | one entry per service that mounts it |
| `writable` | bool | true when any service mounts it RW |
| `state` | `ready` \| `not_created` \| `unresolved` \| `missing` | `not_created`: a named volume that does not exist before the first deploy; `missing`: the volume or path is gone |
| `browsable` | `{ ok: true }` \| `{ ok: false, reason: Reason }` | tmpfs/other: never; bind: `bind_reason()` must not be root or escape |
| `running` | `[service]` | services with a running container that mounts it (FR-010) |

Server-only, never serialised to the UI: `HelperSource::{Volume(name), Bind(PathBuf)}`.

## RelPath

- A relative path inside a mount: `""` means the root.
- **Validation**:
  - UTF-8 and no NUL;
  - components are split on `/`; empty and `.` components are dropped;
  - `..` is rejected (it is never resolved textually);
  - each component is at most 255 bytes, and the whole path at most 4,096 bytes.
- Names that are not valid UTF-8 on disk are listed with `name_lossy`. They can be downloaded and
  deleted by their raw bytes (`name_b64`), but not opened in the editor.

## Entry

| Field | Type | Notes |
|---|---|---|
| `name` | string | lossy display name |
| `name_b64` | string? | present only when the name is not valid UTF-8 |
| `kind` | `file` \| `dir` \| `link` \| `special` | `special` = device, FIFO, socket |
| `link` | `{ target, inside: bool }`? | for links; `inside: false` is never followed (R3) |
| `size` | u64 | bytes (for a link, the target's size when inside) |
| `mtime` | RFC 3339 | |
| `uid`, `gid` | u32 | |
| `mode` | u32 | permission bits; the UI shows `rwxr-x---` |
| `nlink` | u32 | > 1 → `write_blocked: hard_link` |
| `write_blocked` | `hard_link` \| `read_only_mount` \| `special` \| `outside_link` \| null | why writes are off |

Listings are sorted by the server, folders first and then by name (byte order, case-folded for
ASCII). They are sent in pages of 5,000.

## FileContent (ReadFile reply)

| Field | Type | Notes |
|---|---|---|
| `fingerprint` | string | `sha256:<hex>:<size>:<mtime_ns>:<ino>` (R7) |
| `size` | u64 | |
| `text` | string? | present when the content is text and within the editor limit |
| `binary_reason` | `binary` \| `too_large` \| `not_utf8` \| null | |
| `bom` | bool | |
| `eol` | `lf` \| `crlf` \| `mixed` | |
| `read_only` | `WriteBlocked`? | |
| `owner` | `{ uid, gid, mode }` | shown in the editor header |

## FileOp (PreviewFileOp payload)

| Variant | Fields | Preview shows |
|---|---|---|
| `Save` | `mount, path, text, base, bom, eol` | unified diff against `base`, and running services |
| `Create` | `mount, parent, name, kind: file\|dir` | the full path |
| `Rename` | `mount, from, to` | old → new; refused across mounts (there is no field for it) |
| `Delete` | `mount, path` | the name; for a folder, its entry count (bounded walk, "10,000+") |
| `Restore` | `mount, path, version` | diff between that version and the current file |

A confirmed op becomes one helper call. After it, its history capture runs (R9), then its audit
entry is written.

## FileVersion (history)

| Field | Type | Rules |
|---|---|---|
| `id` | string | blob hash plus a sequence |
| `at` | RFC 3339 | |
| `author` | user id | |
| `size` | u64 | ≤ `DEPTHS_FILES_HISTORY_FILE_MAX` (10 MiB) |
| `reason` | `save` \| `upload` \| `delete` \| `restore` | the operation that replaced this content |
| key | `(stack, mount_id, path)` | |

**Retention**: at most 20 per file, 30 days, 1 GiB in total. The oldest are dropped first, after
each write and at start.

## FileAuditEvent

This is an `AuditEntry` with `action` = `files.<op>`, where op is one of `read`, `download`,
`upload`, `save`, `create`, `rename`, `delete`, `restore` or `refused`.

- Fields: `stack`, `service` (the first service that mounts it), `mount` (id and source), `path`,
  `bytes` and `outcome`, plus `reason` when refused.
- Listing folders is not audited, to keep the log readable. Refused listings are.
- The entry never holds content.

## State: an open file in the UI

```
closed → loading → open(clean) ⇄ open(dirty) → previewing → saving → open(clean)
                                     │                        └→ conflict(changed_on_disk) → reload | compare → previewing
                                     └→ leave? → confirm discard
```
