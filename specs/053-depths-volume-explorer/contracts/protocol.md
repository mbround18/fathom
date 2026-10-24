# Contract: protocol additions (053)

`PROTOCOL_VERSION` stays at 1. Every addition is a new variant, so older clients ignore it.
Bindings are regenerated with `cargo test -p depths-protocol --test export_bindings`.

## Commands (WS)

| Cmd | Fields | Reply events | Gate (R12) |
|---|---|---|---|
| `ListMounts` | `stack` | `Mounts { stack, mounts: [Mount] }` | edit; allowed when restricted |
| `ListDir` | `stack, mount, path, req` | `DirPage { req, path, entries, page, last }`, sent 1..n times | edit; allowed when restricted |
| `ReadFile` | `stack, mount, path, req` | `FileContent { req, ... }` | edit; not when restricted; audited `files.read` |
| `FolderSize` | `stack, mount, path, req` | `FolderSize { req, bytes, entries, capped }` | edit; not when restricted |
| `PreviewFileOp` | `stack, op: FileOp` | `FilePreview { preview, stack, op_summary, diff?, entries?, running }` | edit + mode `full`; not when restricted |
| `Confirm` | `preview` (existing) | `FileOpDone { preview, path, fingerprint?, running }` or `Refused` | as for the preview |
| `ListHistory` | `stack, mount, path` | `History { versions: [FileVersion] }` | edit; not when restricted |
| `ReadVersion` | `stack, mount, path, version, req` | `FileContent` (read-only) | edit; not when restricted |

`req` is a client-chosen u32. Replies to a superseded `req` are dropped by the UI (constitution V).
A preview expires after 5 minutes, or when the session ends.

## Refusal kinds (added to the existing `Refused { kind, message }`)

`feature_off`, `feature_read_only`, `not_browsable`, `read_only_mount`, `outside_mount`,
`not_created`, `missing`, `not_confirmed`, `mixed_owners`, `partial` (`removed`, `failed_at`, `errno`, `message`),
`write_blocked` (`hard_link`/`special`/`outside_link`), `changed_on_disk` (with `current`
fingerprint), `owner_changed`, `too_large`, `not_text`, `os_error` (`errno` name and the message,
verbatim), `helper_unavailable` (reason), `untrusted_network` (existing), `denied` (existing).

## `/api/me`

`features.files: "off" | "read_only" | "full"`.

`features.files_reason?: string` is shown to admins when the mode is forced off (no helper image,
or `openat2` is missing). Limits are exposed so the UI can pre-check them:
`limits.files { editor_max, download_max, upload_max }`.

## HTTP

Both routes sit under `/api/v1`, behind the session layer and `observe`. Download responses carry
`Cache-Control: no-store`, `X-Content-Type-Options: nosniff` and `Content-Disposition: attachment`.

| Method | Path | Query | Success | Errors |
|---|---|---|---|---|
| GET | `/stacks/{stack}/files/download` | `mount, path` | 200; a file streams its bytes, a folder streams `application/x-tar` named `<folder>.tar` | 403 `{kind}` gate; 404 missing; 413 `too_large` (with `bytes`) |
| POST | `/stacks/{stack}/files/upload` | `preview` (a confirmed `Upload` preview), `name` | 201 `{ fingerprint }` | 403 `{kind}` (incl. `not_confirmed`); 409 `exists` or `changed_on_disk`; 413; 400 invalid name |

- **Every upload is confirmed first** (constitution I). The client sends `PreviewFileOp{ Upload { mount, dir, files: [{name, size}] } }`; the preview names the folder and every file, marking each name that exists with its current fingerprint. After `Confirm`, the client POSTs one request per file with that preview id. Mount, folder and the `replace` fingerprint come from the preview, never the query. Each name is accepted once, its size must match, and the preview expires like any other.
- Upload is behind `CsrfLayer` (header token plus Origin), and the body is raw bytes
  (`application/octet-stream`). A batch of files is one request per file.
- The route's body limit is `upload_max`. It replaces the global `MAX_BODY_BYTES` for this route
  only.
- An upload that replaces a file keeps history as a save does.
