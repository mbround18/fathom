# Implementation Plan: Depths volume explorer

**Branch**: `050-depths-stack-manager` | **Date**: 2026-10-24 | **Spec**: [spec.md](spec.md)

## Summary

Add a **Files** tab to the stack page. It lists every mount of every service and lets editors
browse, read, edit, download, upload, create, rename, delete and restore files in them.

Depths never touches a volume itself. For each operation it starts a **volume helper**: a
container made from Depths' own image, created through the socket proxy it already uses. The
helper:

- sees only the one mount, at `/volume`, read-only whenever the operation does not write;
- has no network, a read-only root filesystem and no capabilities;
- runs as the owner of the object it acts on, so the kernel's own permission checks apply;
- resolves every path with `openat2(RESOLVE_BENEATH | RESOLVE_NO_MAGICLINKS)`.

The server decides which mounts exist and are browsable, picks the helper's uid and mount mode,
records audit entries and file history, and is the only place a mount's host source comes from.
The client names a mount by its id, never by a path.

Changes keep Depths' preview → confirm flow (constitution I): a save shows its diff, and deletes
and replacements name what they affect. Transfers stream over two new HTTP routes behind the
existing session and CSRF layers.

## Technical Context

**Language/Version**: Rust 2024 (stable), TypeScript 5 / React 19.

**Primary Dependencies**: the existing stack, plus:
- `rustix` (fs: `openat2`, `statx`, `renameat`; already in the lockfile transitively), helper only;
- `tar` 0.4 (already in the lockfile), helper only;
- CodeMirror language packs: `@codemirror/lang-json`, `@codemirror/lang-xml`,
  `@codemirror/legacy-modes` (properties, TOML, shell, Lua, INI/CFG).

**Storage**: file history under `<data_dir>/file-history/`, with content-addressed blobs and one
`meta.json` per tracked file. Writes are atomic, following the `users.rs` pattern. Pruning is
bounded by count, total size and age (R9).

**Testing**:
- `cargo test -p depths-volumes`: path rules, frame codec, text codec, history policy.
- `cargo test -p depths-volume-helper`: the escape suite on a tmpfs tree, with real `openat2`.
- `cargo test -p depths-server`: permission, trust gate, feature mode, audit, history, routes, with
  a fake helper port.
- Vitest: explorer logic (sort, filter, language pick, unsaved guard).
- Playwright: a `us6-files` project against the fake backend.
- A docker-gated integration test that runs the real helper on a scratch volume. It is opt-in
  (`DEPTHS_IT_DOCKER=1`), never in subagent runs.

**Target Platform**: the Depths server container. The kernel must be 5.6 or later (`openat2`),
and Docker 20.10 or later (its default seccomp allows `openat2`).

**Constraints**: FR-020 means no new mounts or capabilities on the Depths container, and the
socket-proxy flags do not change. Downloads stream, with at most 64 MiB of extra RSS (SC-006). The
helper is the only code that opens volume paths.

**Performance Goals**:
- A warm folder listing in 300 ms or less.
- 50,000 entries in under 2 s, including the helper's start (SC-002).
- No input waits longer than 100 ms while scrolling or filtering.

**Scale/Scope**: stacks with up to around 20 mounts, and folders of up to 100k entries. Helpers
are capped at 8 at once per server (R4).

## Constitution Check

*GATE: before Phase 0, and again after Phase 1.*

| Principle | Check | Status |
|---|---|---|
| I. Explicit mutation | Save, create, rename, delete, upload-replace and restore each go through `PreviewFileOp` → `Confirm`. The preview names the stack, mount and path, with a diff for saves. Nothing is shown changed until the helper reports the result. Errors are passed on verbatim. | PASS |
| II. One contract | New `Cmd`/`Event` variants in `depths-protocol`, with ts-rs bindings regenerated. Transfers are HTTP routes, documented in the contract and typed in the protocol crate. The volume logic lives in a new agnostic crate (`depths-volumes`), with the helper and docker as adapters. | PASS |
| III. Boundary tests | Round-trip tests for every new variant. Playwright for US1–US5. Unit tests for the explorer logic. A 30+ case escape suite (SC-003) in the helper crate. | PASS |
| IV. Density | Phone: one pane (list, then file). `md`: list and editor side by side. `xl`: mount tree, list and editor. No sideways page scroll; the list is virtualized. | PASS |
| V. Performance | The listing is virtualized and paged (5,000 entries per event). Helpers are pooled and idle out at 90 s. Nothing polls. | PASS |
| VI. User data | Writes are atomic and keep owner, group and mode. History lives outside the volume. The audit log is append-only and never holds content. Explorer files are never rewritten unasked. | PASS |
| Platforms | Linux container only, as before. Old kernels get the feature off with a stated reason (R3). | PASS |

Re-check after Phase 1: no new violations. The contracts keep every mutation behind `Confirm`, and
the helper's run flags are fixed in the adapter (contracts/helper.md).

## Project Structure

### Documentation (this feature)

```text
specs/053-depths-volume-explorer/
├── plan.md  research.md  data-model.md  quickstart.md
└── contracts/
    ├── protocol.md   # Cmd/Event variants, HTTP transfer routes, refusals
    ├── helper.md     # helper run flags and frame protocol
    └── config.md     # DEPTHS_FEATURE_FILES and limits
```

### Source Code

```text
crates/depths-volumes/                 # agnostic core: no I/O
  src/lib.rs        mount.rs           # Mount, MountId, browsable(), mode per op (ro/rw)
  src/path.rs                          # RelPath: validated relative path; display; parent/join
  src/frame.rs                         # length-prefixed frame codec shared by helper and server
  src/op.rs                            # Op/Reply enums sent to the helper
  src/text.rs                          # UTF-8/BOM/EOL detection and re-application, fingerprints
  src/history.rs                       # retention policy (pure: what to drop)
  src/lang.rs                          # format guess for parse checks (json/yaml/toml/xml/ini)
crates/depths-core/src/ports.rs        # + VolumeHelper port (start(mount, uid, mode) -> channel)
crates/depths-core/src/security.rs     # + Mount.name; mount browsability from bind_reason()
crates/depths-runtime-cli/src/helper.rs  # adapter: `docker run -i` argv with the fixed flags, pool
apps/depths/volume-helper/             # binary crate `depths-volume-helper` (static, in the image)
  src/main.rs  ops.rs  resolve.rs      # openat2 walk, list, read, write, tar, untar, create, ...
apps/depths/server/src/files.rs        # mounts of a stack, op dispatch, gates, audit, previews
apps/depths/server/src/file_history.rs # store + prune
apps/depths/server/src/http.rs         # + GET download, POST upload routes
apps/depths/server/src/{config,ws}.rs  # feature mode, limits; dispatch
crates/depths-protocol/src/{cmd,event,types}.rs
apps/depths/Dockerfile                 # build and copy depths-volume-helper
apps/depths/web/src/features/files/    # FilesTab, MountList, Listing (virtual), FileEditor,
                                       # HistoryPanel, transfers, model.ts, store.ts
apps/depths/web/src/features/editor/   # CodeEditor gains a language and readOnly prop
apps/depths/web/tests/e2e/us6-files/   # browse, edit, transfer, manage, history, refusals
docs/guides/depths/files.md            # user guide; security.md and install.md updated
```

**Structure Decision**: one agnostic crate (`depths-volumes`) shared by the helper binary and the
server, so the frame protocol and path rules exist once. The helper is its own binary in
`apps/depths/`, shipped in the Depths image. Docker stays behind the `ContainerRuntime`-style port
in `depths-core`, implemented in `depths-runtime-cli`.

## Complexity Tracking

| Addition | Why | Simpler alternative rejected |
|---|---|---|
| A helper container per mount and owner | FR-020 and FR-021: no standing host access, and kernel-enforced owner permissions | Mounting the stacks dir and Docker volumes into Depths: standing access to every stack, and it cannot reach named volumes without `/var/lib/docker` |
| A second binary in the image | The helper must be tiny, with no shell, and must not carry the server | Running the server binary with a `helper` subcommand: a larger attack surface in every helper |
| The `depths-volumes` crate | One codec and one path rule set for both sides | Duplicating them in the helper and the server: they would drift, and the drift is a security bug |
| Extra CodeMirror language packs | FR-006 formats | Plain text only: fails FR-006 |
