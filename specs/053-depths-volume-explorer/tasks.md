---

description: "Task list for the Depths volume explorer (053)"
---

# Tasks: Depths volume explorer

**Input**: Design documents from `/specs/053-depths-volume-explorer/`

**Prerequisites**: [plan.md](./plan.md), [spec.md](./spec.md), [research.md](./research.md),
[data-model.md](./data-model.md), [contracts/protocol.md](./contracts/protocol.md),
[contracts/helper.md](./contracts/helper.md), [contracts/config.md](./contracts/config.md),
[quickstart.md](./quickstart.md)

**Tests**: REQUIRED. Constitution Principle III is non-negotiable:
- each new protocol variant gets a round-trip test;
- each user story gets Playwright E2E against the scripted fake backend (`us6-files` project);
- each rule (path containment, gates, metadata keeping, text codec, retention) gets a test that
  fails without it, written before the code.

**Organization**: Setup and Foundational build the agnostic core, the helper's path resolver, the
docker adapter, the gates and the empty Files tab. Each user-story phase then adds one slice
end to end: helper op → server dispatch → protocol → UI → tests.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: can run in parallel: different files, and no dependency on an incomplete task.
- **[Story]**: US1–US5, used in the user-story phases only.

## Rules for every task in this file

- **Builds and tests**: `CARGO_BUILD_JOBS=4 nice`, one heavy command at a time, `cargo check -p
  <crate>` while iterating. Gate on the targeted tests named in the task and in quickstart.md.
- **Subagents never run `scripts/ci.sh`, `paws` or `dagger`.** The main session runs the full gate
  once, on the combined branch.
- **Docker**: only T073 (the opt-in `DEPTHS_IT_DOCKER=1` test) touches a real daemon. It removes
  what it created by name (`depths-it-<random>` volume, its temp dir, `depths.helper` containers it
  started). Never prune, never remove an image, never touch another container or volume.
- **Protocol changes**: after any edit to `crates/depths-protocol`, run
  `cargo test -p depths-protocol --test export_bindings` and commit the regenerated bindings with
  the change.
- **Paths are never logged or put in span attributes** (R15). The audit entry carries the path; the
  span does not.
- **Commits**: one signed commit per task or tight group of tasks.

---

## Phase 1: Setup

- [X] T001 Create the agnostic crate `crates/depths-volumes` (`Cargo.toml` with `edition = "2024"`, `license = "Apache-2.0"`, `publish = false`; deps `serde`, `serde_json`, `sha2`, `thiserror`, `ts-rs` behind the same feature `depths-protocol` uses; no tokio, no fs I/O) with empty modules `mount`, `path`, `frame`, `op`, `text`, `history`, `lang` in crates/depths-volumes/src/lib.rs. It is picked up by the existing `crates/*` workspace member.
- [X] T002 Create the binary crate `depths-volume-helper` in apps/depths/volume-helper/ (`src/main.rs`, `src/resolve.rs`, `src/ops.rs`; deps `depths-volumes`, `rustix` with `fs` feature, `tar` 0.4, `sha2`; no async runtime, no network crates) and add `"apps/depths/volume-helper"` to the root `Cargo.toml` `[workspace] members`.
- [X] T003 [P] Add `DEPTHS_FEATURE_FILES` (`off`|`read_only`|`full`, default `full`), `DEPTHS_HELPER_IMAGE`, `DEPTHS_FILES_EDITOR_MAX` (2MiB), `DEPTHS_FILES_DOWNLOAD_MAX` (2GiB), `DEPTHS_FILES_UPLOAD_MAX` (1GiB), `DEPTHS_FILES_HISTORY_FILE_MAX` (10MiB), `DEPTHS_FILES_HISTORY_VERSIONS` (20), `DEPTHS_FILES_HISTORY_TOTAL` (1GiB), `DEPTHS_FILES_HISTORY_DAYS` (30), `DEPTHS_FILES_HELPERS` (8) to apps/depths/server/src/config.rs, parsed with the same size units as `DEPTHS_MAX_FILE_BYTES`; an invalid value fails start naming the key. Raise the WS message cap to `max(max_message_bytes(max_file_bytes), editor_max + 64 KiB)`. Unit tests for each default and one invalid value per key in the same file.
- [X] T004 [P] Add the web dependencies `@codemirror/lang-json`, `@codemirror/lang-xml`, `@codemirror/legacy-modes`, `smol-toml` to apps/depths/web/package.json with `pnpm --filter @companion/depths-web add`, and confirm `cargo deny`-style licence allow-list coverage is unchanged (all MIT/BSD-3).
- [X] T005 [P] Register the Playwright project `us6-files` in apps/depths/web/playwright.config.ts (append to `STORIES`) and create apps/depths/web/tests/e2e/us6-files/ with a placeholder spec that only opens the stack page.

---

## Phase 2: Foundational (blocks every story)

**Purpose**: the core types and rules, the helper's containment, the docker adapter, the gates, the
mount list and an empty Files tab. No story can start until this phase is done.

### Core crate (tests first)

- [X] T006 [P] Write failing tests, then implement `RelPath` in crates/depths-volumes/src/path.rs. Rules quoted from data-model.md: "UTF-8 and no NUL; components are split on `/`; empty and `.` components are dropped; `..` is rejected (it is never resolved textually); each component is at most 255 bytes, and the whole path at most 4,096 bytes"; `""` is the root. Provide `parent()`, `join(name)` (name validated as one component), `components()`, `Display`. Tests: every rule, a leading `/` rejected, `a//./b` → `a/b`, `a/../b` rejected.
- [X] T007 [P] Write failing tests, then implement the frame codec in crates/depths-volumes/src/frame.rs per contracts/helper.md: `frame = len:u32be kind:u8 body[len]`, kinds `0 Hello | 1 Op | 2 Reply | 3 Data | 4 End | 5 Error`, max len 1 MiB. Blocking `read_frame<R: Read>` / `write_frame<W: Write>`; a length over 1 MiB, an unknown kind or a short read is an error. Tests: round trip of each kind, oversized length, truncated body, unknown kind.
- [X] T008 [P] Define the helper op/reply types in crates/depths-volumes/src/op.rs (serde JSON): `Hello { proto: 1, openat2: bool, uid, gid }`; ops `Stat`, `List { path, page }`, `Read { path, max }`, `Write { path, base, mode? }`, `Create`, `Rename`, `Delete { path, recursive }`, `Size { path, cap }`, `Tar { path }`, `Untar` (declared, not implemented), `Put { dir, name, replace }`; replies including `Reply { fingerprint, prior: bool }`; error kinds `outside_mount`, `write_blocked{hard_link|special|outside_link}`, `changed_on_disk{current}`, `owner_changed`, `too_large`, `not_text`, `os_error{errno, message}`. JSON round-trip tests for every variant in the same file.
- [X] T009 Write failing tests, then implement `Entry`, `FileContent`, `Fingerprint` and listing order in crates/depths-volumes/src/mount.rs and crates/depths-volumes/src/text.rs. Entry fields per data-model.md (`name`, `name_b64` "present only when the name is not valid UTF-8", `kind` `file|dir|link|special`, `link {target, inside}`, `size`, `mtime` RFC 3339, `uid`, `gid`, `mode`, `nlink`, `write_blocked` `hard_link|read_only_mount|special|outside_link|null`). Sort "folders first and then by name (byte order, case-folded for ASCII)"; pages of 5,000. Fingerprint format `sha256:<hex>:<size>:<mtime_ns>:<ino>` with parse and equality. This task owns the first version of mount.rs and text.rs; T010 and T011 start after it.
- [X] T010 [P] Write failing property tests, then implement the text codec in crates/depths-volumes/src/text.rs (R8): text iff valid UTF-8, no NUL in the first 8 KiB, and within the editor limit, else `binary_reason` `binary|too_large|not_utf8`; strip and flag a BOM; detect `eol` `lf|crlf|mixed`; `decode` gives LF text, `encode(text, bom, eol)` re-applies them; `mixed` encodes with the most common ending; trailing newline kept as is. Property test: `encode(decode(x)) == x` for every non-mixed input.
- [X] T011 [P] Implement `Mount`, `MountId` and browsability in crates/depths-volumes/src/mount.rs: `MountId` = first 16 hex of `sha256(kind \0 source)`; `kind` `volume|bind|tmpfs|other` (empty read as `bind`); `state` `ready|not_created|unresolved|missing`; `browsable` `{ok:true}|{ok:false, reason}` (tmpfs/other never; a bind needs a caller-supplied rating of info or none); `writable` = any service RW; `op_mode(op) -> ReadOnly|ReadWrite`. Tests for id stability, empty-kind reading, every browsable rule. `HelperSource::{Volume(name), Bind(PathBuf)}` lives here, server-only (not ts-rs exported).
- [X] T012 [P] Add `name` to `security::Mount` (inspect `Name`) in crates/depths-core/src/security.rs, with a fixture test that a named volume's name is read and a bind has none; keep every existing 051 rating test passing.

### Helper containment (the security core)

- [X] T013 Write the escape suite first (it must fail to compile/run against a stub), in apps/depths/volume-helper/tests/escape.rs: build a tree in a `tempfile` dir with a sentinel file planted beside (outside) the root, and at least 30 cases (SC-003): `..` in many positions, absolute paths, relative links out, absolute links (even ones that would land inside), link chains, a link swapped to point outside concurrently with a read and with a write (thread loop), a dir swapped for a link mid-walk, `/proc/self/root`-style magic links, hard links (write refused `hard_link`), FIFO/socket/device (refused `special`), a non-UTF-8 name addressed by bytes. Every case either acts inside the root or is refused `outside_mount`/`write_blocked`; afterwards the sentinel's content, mtime and inode are unchanged. The same case table is reused by T030 through `FakeVolumeHelper` to assert exactly one `files.refused` audit entry per refused case (SC-003).
- [X] T014 Implement the resolver in apps/depths/volume-helper/src/resolve.rs (R3): open `/volume` (or the test root) as a dir fd; `openat2(root, path, RESOLVE_BENEATH | RESOLVE_NO_MAGICLINKS)` for the parent; open the final component with `O_NOFOLLOW` against the parent `O_PATH` fd and `statx` it; map `EXDEV`/`ELOOP` from a link out to `outside_mount`; never follow absolute links; refuse writes when `nlink > 1` or the object is special. Probe `openat2` at start (`ENOSYS` → `Hello.openat2 = false`). Make T013 pass.
- [X] T015 Implement the helper main loop in apps/depths/volume-helper/src/main.rs: send `Hello`, then read one `Op` frame at a time and dispatch to `ops.rs`; exit 0 on stdin EOF, 70 on a protocol error; no stdout output other than frames; root from `DEPTHS_HELPER_ROOT` only under `#[cfg(test)]`/a test-only flag, `/volume` otherwise. Implement `Stat` in apps/depths/volume-helper/src/ops.rs. Test (spawning the built binary against a temp root): handshake, `Stat` of root, protocol error exits 70.

### Docker adapter and port

- [X] T016 Add the `VolumeHelper` port to crates/depths-core/src/ports.rs: `start(source: HelperSource, uid, gid, mode, stack) -> Result<HelperChannel>` where `HelperChannel` exposes async frame send/receive and `close()`; plus a `FakeVolumeHelper` behind `cfg(any(test, feature = "fake"))` that runs the helper op logic in-process against a temp root, for server tests.
- [X] T017 Write failing argv tests, then implement the adapter in crates/depths-runtime-cli/src/helper.rs: build exactly the argv in contracts/helper.md (`docker run -i --rm --pull never --network none --read-only --cap-drop ALL --security-opt no-new-privileges --user <uid>:<gid> --memory 256m --pids-limit 32 --cpus 1 --log-driver none --label depths.helper=1 --label depths.helper.stack=<stack> --entrypoint /usr/local/bin/depths-volume-helper --mount ...[,readonly] <image id>`). Refuse before building it: a named volume the caller has not confirmed exists (the adapter takes a `VolumeExists` proof from `docker volume inspect`, so it can never create one); a volume name failing Docker's name rule; a bind source that is not absolute, not normalised, contains `,` or `=`, or is rated root/escape by `bind_reason()`. `readonly` unless the op writes. uid/gid numeric only. Tests assert the full argv for a volume and a bind, ro and rw, and each refusal.
- [X] T018 Implement the helper pool in crates/depths-runtime-cli/src/helper.rs (R4): a warm helper per `(mount, uid, gid, mode)`, 90 s idle timeout, at most `files_helpers` (8) live, LRU closed first, one op at a time per helper (serialised), one-shot helpers for transfers; kill and `docker rm -f` by id on protocol error. On server start remove leftover containers labelled `depths.helper=1`. Unit-test eviction and idle timeout with a fake clock and a fake spawner.
- [X] T019 Resolve the helper image at start in apps/depths/server/src/files.rs (R2): `DEPTHS_HELPER_IMAGE`, else inspect own container via `HOSTNAME` and take its image ID; run by image ID only. If neither works, or the first helper's `Hello.openat2` is false, force the feature `off` with `files_reason` ("no helper image" / "kernel older than 5.6 or seccomp blocks openat2"). Test with the fake runtime for each outcome.

### Protocol, gates, audit

- [X] T020 Add to crates/depths-protocol/src/{cmd,event,types}.rs, re-exporting the `depths-volumes` types: Cmds `ListMounts`, `ListDir`, `ReadFile`, `FolderSize`, `PreviewFileOp`, `ListHistory`, `ReadVersion` (fields per contracts/protocol.md, `req: u32` where listed); Events `Mounts`, `DirPage { req, path, entries, page, last }`, `FileContent`, `FolderSize { req, bytes, entries, capped }`, `FilePreview`, `FileOpDone`, `History`; `FileOp` variants `Save { mount, path, text, base, bom, eol }`, `Create { mount, parent, name, kind: file|dir }`, `Rename { mount, from, to }`, `Delete { mount, path }`, `Restore { mount, path, version }`, `Upload { mount, dir, files: [{ name, size }] }` (its preview returns, per name, `exists` and the current `fingerprint` of the file it would replace, and a `preview` id the upload POST must carry); the new refusal kinds. `PROTOCOL_VERSION` stays 1. Round-trip tests for every new variant in crates/depths-protocol/tests/roundtrip.rs; regenerate bindings.
- [X] T021 Add the new command kinds to `CommandKind` and `restricted_ok` in crates/depths-core/src/role.rs: `FilesList` (ListMounts, ListDir; allowed when restricted), `FilesRead` (ReadFile, FolderSize, ListHistory, ReadVersion, download), `FilesWrite` (PreviewFileOp/Confirm, upload); all need edit permission. Tests: viewer refused all three; `restricted_ok` true only for `FilesList`.
- [X] T022 Write failing tests, then implement `files_gate(session, ip, stack, kind, mount)` in apps/depths/server/src/files.rs (R12), in order: feature mode (off → `Denied feature_disabled`; read_only → only list/read/download, else `feature_read_only`) → `role::permits` edit (`Denied not_permitted`) → 052 trust (`Denied untrusted_network` unless `restricted_ok`) → mount state `ready` (else `not_created` or `missing`: `docker run --mount type=volume` would otherwise create an empty, unlabelled volume) → mount browsable (`not_browsable`) and writable for writes (`read_only_mount`). Every refusal writes one `files.refused` audit entry with `reason`. Also a `refuse_from_helper(err)` used by every dispatch path: each helper error (`outside_mount`, `write_blocked`, `owner_changed`, `changed_on_disk`, `too_large`, `not_text`, `os_error`, `partial`) becomes the matching `FileRefused` to the client and one `files.refused` audit entry with the op, path and reason. Tests in apps/depths/server/tests/files.rs cover every branch, including one helper error of each kind.
- [X] T023 [P] Add the `files.<op>` audit actions (`read`, `download`, `upload`, `save`, `create`, `rename`, `delete`, `restore`, `refused`) in apps/depths/server/src/audit.rs with fields `stack`, `service` (first service that mounts it), `mount` (id and source), `path`, `bytes`, `outcome`, `reason?`; a test asserts no field can carry content. Add the OTel span `files.op` (kind, stack, mount id, outcome, bytes, cold/warm; no path) and histogram `depths.files.helper_start_ms` in apps/depths/server/src/telemetry.rs.
- [X] T024 Implement mount discovery in apps/depths/server/src/files.rs (R10): from the stack's inspected containers' `Mounts` (with the new `name`); for services with no container, from `docker compose config --format json` (argv like the existing `config --quiet` step), bind sources resolved relative to the stack dir and unresolvable ones `unresolved`; missing named volumes `not_created`, and a volume or path gone since discovery `missing` (re-checked with `docker volume inspect` / a stat of the bind's parent before each helper start); mounts shared by services merged with one `services` entry each; `running` = services with a running container mounting it; browsability from `bind_reason()`. Server-side map `MountId → HelperSource`; the client never supplies a source. Tests with inspect and compose-config fixtures: volume, relative bind, `/` bind (not browsable, reason), tmpfs, shared mount, never-deployed stack.
- [X] T025 Expose `features.files` (`off|read_only|full`), admin-only `features.files_reason`, and `limits.files { editor_max, download_max, upload_max }` in `/api/me` in apps/depths/server/src/http.rs; extend the existing `/api/me` test.

### UI shell and fake backend

- [X] T026 [P] Extend the fake backend in apps/depths/web/tests/support/fake-backend.ts with an in-memory volume model: mounts per stack, folders/files with uid/gid/mode/nlink, fingerprints, a running flag, a seedable large folder, scripted refusals, and handlers for every new Cmd (replies paged at 5,000) plus the two HTTP routes; `features.files` and limits in `/api/me`.
- [X] T027 Add the Files tab to the stack page in apps/depths/web/src/features/stacks/StackDetail.tsx, lazy-loading `features/files/FilesTab.tsx` with `React.lazy`; shown only when `features.files !== 'off'` and the person may edit the stack (the same check as the Edit button). URL `…/stacks/<stack>/files/<mountId>/<path>` (FR-024), parsed and built in apps/depths/web/src/features/files/model.ts with unit tests in apps/depths/web/tests/unit/files.test.ts.
- [X] T028 Add `language` and `readOnly` props to apps/depths/web/src/features/editor/CodeEditor.tsx (compose editor unchanged by default), and apps/depths/web/src/features/files/lang.ts mapping names and content sniffing to lazy loaders for INI/CFG, JSON, YAML, TOML, XML, properties, shell, Lua, plain text. Unit tests for the mapping (`server.properties`, `adminlist.txt`, `*.cfg`, `start_server.sh`, a `#!/bin/sh` file without extension, `config.yml`).

**Checkpoint**: escape suite green, gates tested, the Files tab appears (empty) for editors and never for viewers.

---

## Phase 3: User Story 1 - Browse a service's volumes and read a file (Priority: P1) 🎯 MVP

**Goal**: an editor opens the Files tab, sees every mount, walks folders and reads a text file in the editor; binary files offer download.

**Independent Test**: seed a stack with a named volume and a bind mount holding text, binary and a subfolder; open the Files tab, walk in, open a text file. Listing and contents match; a binary shows size and type with a download button; a viewer has no tab and is refused.

### Tests for User Story 1

- [X] T029 [P] [US1] Helper tests in apps/depths/volume-helper/tests/ops_read.rs: `List` sorted folders-first, paged at 5,000, non-UTF-8 names as `name_lossy` + `name_b64`, links with `{target, inside}`, `write_blocked` set for hard links/special/outside links; `Read` returns the fingerprint and content up to `max`, `too_large` above it; a 50,000-entry folder lists in under 1 s locally.
- [X] T030 [P] [US1] Server tests in apps/depths/server/tests/files.rs with `FakeVolumeHelper`: `ListMounts` shapes, `ListDir` pages and `req` echo, `ReadFile` text/binary/too_large and its `files.read` audit; the helper runs as the folder's/file's owner (R5) and as `65534:65534` for the mount root stat; `owner_changed` when the owner moved; the T013 case table run through the server yields one `files.refused` entry per refused case; viewer refused; restricted view: `ListMounts`/`ListDir` pass, `ReadFile` refused `untrusted_network`.
- [X] T031 [P] [US1] Playwright in apps/depths/web/tests/e2e/us6-files/browse.spec.ts: mounts listed with type, container path and ro state; walk into a subfolder and back to the root, never above it; open a text file with line numbers and highlighting; binary file → size, type, download; viewer sees no Files tab; never-deployed stack explains named volumes appear after deploy; outside link shown with target and not openable; `/` bind listed with the reason and not browsable; 50,000-entry folder visible in under 2 s and filter responds under 100 ms (SC-002); 360 px wide has no sideways scroll; deep link reload reopens the same file.

### Implementation for User Story 1

- [X] T032 [US1] Implement `List` and `Read` in apps/depths/volume-helper/src/ops.rs on fds from `resolve.rs`, `statx` for every entry, streaming `Read` content as Data frames up to `max`, fingerprint computed on the open fd (R7).
- [X] T033 [US1] Dispatch `ListMounts`, `ListDir`, `ReadFile` in apps/depths/server/src/ws.rs through `files_gate`, picking the helper uid:gid from the listing that led to the op (R5) and the read-only mode; decode with `depths-volumes::text`; audit `files.read`. Add `GET /api/v1/stacks/{stack}/files/download?mount=&path=` for **files** in apps/depths/server/src/http.rs (session layer, `observe`, `files_gate` as `FilesRead`, one-shot helper, streamed Data frames with backpressure, `Cache-Control: no-store`, `X-Content-Type-Options: nosniff`, `Content-Disposition: attachment`, 413 `too_large` above `download_max`, audit `files.download` with size); a folder path returns 400 until T050.
- [X] T034 [P] [US1] Client store for the explorer in apps/depths/web/src/features/files/store.ts: current mount/path, pages appended per `req`, replies to a superseded `req` dropped, name filter, open file state machine per data-model.md (`closed → loading → open(clean)`). Unit tests in apps/depths/web/tests/unit/files.test.ts for sort-stable paging, superseded replies and filter.
- [X] T035 [P] [US1] `MountList` in apps/depths/web/src/features/files/MountList.tsx: per service, name or source (source shown to editors only), type, container path, ro state, state (`not_created` explanation, `unresolved`, `missing`), not-browsable reason.
- [X] T036 [P] [US1] Virtualized `Listing` in apps/depths/web/src/features/files/Listing.tsx: breadcrumb stopping at the mount root, name filter, columns name/size/mtime/owner/`rwxr-x---`, link targets, special files inert, keyboard navigation, touch targets per the density rules.
- [X] T037 [US1] `FileEditor` (read path) in apps/depths/web/src/features/files/FileEditor.tsx: header with path, owner and mode; `CodeEditor` with the guessed language, search, soft wrap; `readOnly` with the reason when the mount is read-only everywhere or `write_blocked`; binary/too_large view with size, type and a download link to the file route from T033.
- [X] T038 [US1] Compose `FilesTab` in apps/depths/web/src/features/files/FilesTab.tsx with the layout rules (FR-023): phone one pane (list, then file full width), `md` list and editor side by side, `xl` mount tree + list + editor.

**Checkpoint**: US1 is shippable alone as a read-only explorer with file downloads (folder archives come in US3).

---

## Phase 4: User Story 2 - Edit and save a configuration file (Priority: P1)

**Goal**: save an edited file atomically, keeping owner, group, mode, BOM and line endings; refuse a save over a changed file; offer save and restart when the service runs.

**Independent Test**: change a line in a writable mount and save; disk has the new content with the same uid/gid/mode and an audit entry; a second save after an on-disk change is refused with reload/compare.

### Tests for User Story 2

- [X] T039 [P] [US2] Helper tests in apps/depths/volume-helper/tests/ops_write.rs (SC-004): `Write` keeps uid, gid and mode (run as the current user on a temp tree with mode 0640); stale `base` → `changed_on_disk { current }`; a write whose Data stream is cut mid-way (helper killed) leaves the old content and no `.depths-save-*` file; a parent the owner cannot write → `os_error` verbatim and no change; `prior` frames return exactly the replaced bytes when ≤ 10 MiB and `prior: false` above.
- [ ] T040 [P] [US2] Server tests in apps/depths/server/tests/files.rs: `PreviewFileOp{Save}` returns a unified diff and `running`; `Confirm` performs one helper call and audits `files.save`; `read_only` mode → `feature_read_only`; a read-only mount → `read_only_mount`; CRLF and BOM files round-trip byte-exact when only one line changed; the preview expires after 5 minutes and on session end.
- [ ] T041 [P] [US2] Playwright in apps/depths/web/tests/e2e/us6-files/edit.spec.ts: edit, preview diff, confirm, content shown saved; conflict offers reload (edit lost) or compare then save over; running service warns and offers **Save and restart <service>** only with restart permission, restart goes through the ordinary `PreviewOp{restart}` → `Confirm`; invalid JSON/YAML/TOML/XML/INI is marked with its line and save still works after a confirm; read-only mount opens read-only with the reason; leaving with unsaved changes asks to confirm.

### Implementation for User Story 2

- [X] T042 [US2] Implement `Write` in apps/depths/volume-helper/src/ops.rs (R6): open parent beneath the root; `O_EXCL` create `.depths-save-<random>`; write Data frames; `fchmod` to the old mode; recompute the fingerprint of the target on its open fd and compare with `base` immediately before `renameat`; `fsync` file, `renameat` over the target, `fsync` parent; unlink the temp on any failure; send `prior` frames when ≤ history file max. Never truncate in place.
- [ ] T043 [US2] Server `PreviewFileOp{Save}` → `Confirm` in apps/depths/server/src/files.rs: encode with `text::encode(text, bom, eol)`, diff against `base` with the existing apps/depths/server/src/diff.rs, `running` services in the preview, helper started RW as the file's owner, `FileOpDone { fingerprint, running }`, audit `files.save`. Hand the `prior` bytes to a `HistorySink` trait that is a no-op until US5.
- [ ] T044 [P] [US2] Browser parse checks in apps/depths/web/src/features/files/lint.ts through `@codemirror/lint`: JSON (native), YAML (`yaml`), TOML (`smol-toml`), XML (`DOMParser`), INI (small hand-written checker for unclosed sections and lines without `=`/`:`). Warnings only. Unit tests per format with a line number.
- [ ] T045 [US2] Save flow in apps/depths/web/src/features/files/FileEditor.tsx: `open(dirty) → previewing → saving → open(clean)`, conflict branch (reload | compare → previewing), mixed-EOL notice, parse-warning confirm, `useUnsavedGuard`, and the running-service prompt with **Save** / **Save and restart <service>** (shown only if `permits(restart)`), chaining the existing restart preview.

**Checkpoint**: US1 + US2 give the requested feature: browse and edit config files safely.

---

## Phase 5: User Story 3 - Download and upload files (Priority: P2)

**Goal**: stream files and folders out (folder as tar) and files in, within limits, with confirm on replace.

**Independent Test**: download a folder with a subfolder and compare the tar with disk; upload over an existing file: new content, old owner and mode, replaced version restorable once US5 lands.

### Tests for User Story 3

- [X] T046 [P] [US3] Helper tests in apps/depths/volume-helper/tests/ops_transfer.rs: `Tar` of a folder lists the same paths and bytes as disk and never includes outside links' targets; `Size { cap }` stops at the cap and reports `capped`; `Put` writes atomically, refuses an existing name without a matching `replace` fingerprint (`exists` / `changed_on_disk`), keeps the replaced file's uid/gid/mode, rejects an invalid name.
- [ ] T047 [P] [US3] Server tests in apps/depths/server/tests/http.rs: `PreviewFileOp{Upload}` names the folder and each file, marks replacements with their current fingerprint, and expires like other previews; a POST without a confirmed `preview` id, or for a name/size the preview did not cover, is refused 403 `not_confirmed`; download sets `Cache-Control: no-store`, `X-Content-Type-Options: nosniff`, `Content-Disposition: attachment` (`<folder>.tar` for a folder); 403 `{kind}` for viewer/restricted/feature-off; 413 `too_large` with `bytes` above `download_max`; upload needs CSRF header, Origin and a confirmed `preview`, 201 `{fingerprint}`, 409 `exists`/`changed_on_disk`, 413 above `upload_max` while other routes keep `MAX_BODY_BYTES`; audits `files.download`/`files.upload` with size on success and failure; a 1 GiB streamed download through `FakeVolumeHelper` raises RSS by at most 64 MiB (SC-006, measured from `/proc/self/statm`, marked `#[ignore]` unless `DEPTHS_SLOW_TESTS=1`).
- [ ] T048 [P] [US3] Playwright in apps/depths/web/tests/e2e/us6-files/transfer.spec.ts: download a file and a folder (filenames, bytes from the fake), over-limit folder shows its size and starts nothing; upload several files: a confirm names the folder and every file (replacements marked) before any byte is sent, then a progress bar; nothing is written if the confirm is cancelled; a running service warns and offers to stop it first.

### Implementation for User Story 3

- [X] T049 [US3] Implement `Size`, `Tar` (uncompressed, written to stdout as Data frames, entries opened beneath the root) and `Put` (temp + rename per R6) in apps/depths/volume-helper/src/ops.rs.
- [ ] T050 [US3] Extend the download route from T033 to folders (`application/x-tar`, `<folder>.tar`), and add `POST /api/v1/stacks/{stack}/files/upload?preview=&name=` in apps/depths/server/src/http.rs (mount, folder and `replace` fingerprint come from the confirmed `Upload` preview, never from the query; each name is accepted once per preview) behind the session layer and `observe`, upload behind `CsrfLayer` with its own body limit `upload_max`; both use `files_gate` and one-shot helpers; the body streams helper Data frames with backpressure (never buffered); size pre-check with `FolderSize`; `FolderSize` WS dispatch in apps/depths/server/src/ws.rs; replaced uploads hand `prior` to `HistorySink`.
- [ ] T051 [US3] Transfers UI in apps/depths/web/src/features/files/transfers.ts and Listing/FileEditor actions: download links, folder size pre-check, upload picker and drop target → `PreviewFileOp{Upload}` → confirm dialog naming the folder and every file (replacements marked) → one POST per file carrying the preview id, with progress, running-service warning with a stop offer through the ordinary lifecycle preview.

---

## Phase 6: User Story 4 - Create, rename and delete (Priority: P3)

**Goal**: housekeeping inside a writable mount, each behind preview → confirm.

**Independent Test**: create a file and a folder, rename the file, delete both; disk and audit reflect each step; new entries take the parent's owner and group.

### Tests for User Story 4

- [X] T052 [P] [US4] Helper tests in apps/depths/volume-helper/tests/ops_manage.rs: `Create` file/dir owned by the parent's uid/gid (helper runs as it) with default mode; `Rename` within the mount, refused when the target exists or escapes, and refused `mixed_owners` when the source and destination folders have different owners; `Delete` of a file and of a folder with `recursive`, never following links out; a recursive delete that hits an entry the helper's uid cannot remove stops and replies `partial { removed, failed_at, errno, message }`; deleting through a swapped link touches nothing outside (extend the T013 sentinel).
- [ ] T053 [P] [US4] Server tests in apps/depths/server/tests/files.rs: previews for `Create`/`Rename`/`Delete` name the full path, old → new, and a bounded entry count ("10,000+"); helper runs as the parent's owner; `mixed_owners` refused before any helper write; a `partial` delete is audited with what was removed and shown verbatim; audits `files.create`/`files.rename`/`files.delete`; deleted file content goes to `HistorySink`.
- [ ] T054 [P] [US4] Playwright in apps/depths/web/tests/e2e/us6-files/manage.spec.ts: create file and folder, rename, move a file into another folder of the same mount, delete with a confirm naming it and the count; a partial delete shows what was removed and the system error; no cross-mount move offered.

### Implementation for User Story 4

- [X] T055 [US4] Implement `Create`, `Rename` (`renameat` between two fds beneath the root, no replace) and `Delete` (bounded recursive walk on fds) in apps/depths/volume-helper/src/ops.rs.
- [ ] T056 [US4] Server previews and confirms for `FileOp::{Create, Rename, Delete}` in apps/depths/server/src/files.rs, with the entry count from `Size`.
- [ ] T057 [US4] Create/rename/delete actions and confirm dialogs in apps/depths/web/src/features/files/Listing.tsx (context menu and keyboard).

---

## Phase 7: User Story 5 - Undo from history (Priority: P3)

**Goal**: keep the previous content before every Depths write, list it, compare and restore.

**Independent Test**: save a file three times, open history: three versions with time and author; restoring the first puts its exact bytes back and records a new save.

### Tests for User Story 5

- [X] T058 [P] [US5] Write failing tests, then implement the pure retention policy in crates/depths-volumes/src/history.rs: input versions with `(file key, at, size)` plus limits; output what to drop, in order "per-file count, then age, then total size, oldest first" (defaults 20 versions, 30 days, 1 GiB); files over `DEPTHS_FILES_HISTORY_FILE_MAX` (10 MiB) get no version. Tests per limit and their combination.
- [ ] T059 [P] [US5] Server tests in apps/depths/server/tests/files.rs: three saves → three `FileVersion`s newest first with author and `reason` `save`; upload/delete/restore reasons; `ReadVersion` content; `Restore` previews a diff and is itself audited `files.restore` with its own history entry; pruning at start and after each write; orphan blobs removed; a crash between blob and JSON write leaves a consistent store.
- [ ] T060 [P] [US5] Playwright in apps/depths/web/tests/e2e/us6-files/history.spec.ts: open history, compare a version, restore in under 30 s of interaction (SC-005), deleted file restorable from the folder's view while its history is kept.

### Implementation for User Story 5

- [ ] T061 [US5] History store in apps/depths/server/src/file_history.rs (R9): `<data_dir>/file-history/blobs/<sha256>` written temp + rename; `<data_dir>/file-history/files/<sha256(stack \0 mount \0 path)>.json` with versions (`id` blob hash plus sequence, `at`, `author`, `size`, `blob`, `reason`), written atomically following apps/depths/server/src/users.rs; prune with `depths_volumes::history` after each write and at start; remove unreferenced blobs. Implements `HistorySink`.
- [ ] T062 [US5] `ListHistory`, `ReadVersion` and `FileOp::Restore` in apps/depths/server/src/files.rs and dispatch in apps/depths/server/src/ws.rs; restore writes through the same `Write` path with `base` = current fingerprint.
- [ ] T063 [US5] `HistoryPanel` in apps/depths/web/src/features/files/HistoryPanel.tsx: versions newest first, compare against current with the existing diff view, restore through preview → confirm; deleted-file entry point in the listing.

---

## Phase 8: Polish & Cross-Cutting

- [ ] T064 Build and ship the helper in apps/depths/Dockerfile: compile `depths-volume-helper` statically (musl or `-C target-feature=+crt-static`) in the existing cargo-chef builder stage, `COPY` it to `/usr/local/bin/depths-volume-helper` in the runtime image; no shell added. `docker build` locally once (main session only).
- [X] T065 [P] User guide docs/guides/depths/files.md: opening the Files tab, editing and saving, conflicts, save and restart, transfers and limits, history and its limits, what is not browsable and why, xattrs/ACLs not kept on save, the fingerprint race window (R7), kernel ≥ 5.6 requirement, `DEPTHS_FEATURE_FILES`.
- [X] T066 [P] Update docs/guides/depths/security.md (helper container model, FR-020/FR-021, audit actions, restricted view rule for files) and docs/guides/depths/install.md (no new mounts or proxy flags; kernel and Docker version; `DEPTHS_HELPER_IMAGE`).
- [X] T067 [P] Add the 053 settings to apps/depths/.env.example and cross-reference contracts/config.md from specs/050-depths-stack-manager/contracts/config.md.
- [ ] T068 [P] Feature-mode Playwright in apps/depths/web/tests/e2e/us6-files/modes.spec.ts: `off` hides the tab; `read_only` keeps browse/open/download and hides every change control; admin sees `files_reason` when forced off.
- [ ] T069 [P] A11y pass on the explorer: keyboard-only walk through US1–US5, focus return after dialogs, `aria-live` for save results, in apps/depths/web/tests/e2e/us6-files/a11y.spec.ts.
- [ ] T070 Run every quickstart.md section 1–3 command one at a time and fix what fails.
- [ ] T071 Run `scripts/hygiene.sh` and remove anything this work left dangling.
- [ ] T072 Main session only: one full `scripts/ci.sh` on the combined branch.
- [ ] T073 Main session only, opt-in: write and run apps/depths/server/tests/files_docker.rs (`DEPTHS_IT_DOCKER=1`, `DEPTHS_HELPER_IMAGE=depths:dev`) per quickstart.md §4: scratch volume `depths-it-<random>` and bind dir with files `1000:1000 0640`; list, save (still `1000:1000 0640`), tar download, upload, `../`, absolute link, link swap; clean up by name.
- [ ] T074 Brochure/marketing screenshots of the Files tab, last.

---

## Dependencies & Execution Order

### Phase dependencies

- **Setup (T001–T005)**: no dependencies. T003–T005 parallel with T001/T002.
- **Foundational (T006–T028)**: after Setup. Blocks every story.
  - Core crate: T006, T007, T008, T012 parallel; T009 first among the mount.rs/text.rs tasks, then T010 and T011 in parallel.
  - T013 → T014 → T015 (helper containment, one track).
  - T016 after T011; T017 → T018 after T016; T019 after T018.
  - T020 after T006–T011; T021 → T022 after T020; T023 parallel; T024 after T012 + T022; T025 after T003.
  - UI T026–T028 after T020 (bindings), parallel with each other.
- **US1 (T029–T038)**: after Foundational. MVP.
- **US2 (T039–T045)**: after US1 (needs the reader and the editor view).
- **US3 (T046–T051)**: after Foundational; independent of US2 except the `HistorySink` trait (T043) — define it in T043 or stub it in T050, whichever lands first.
- **US4 (T052–T057)**: after US1.
- **US5 (T058–T063)**: after US2 (captures `prior` from writes). T058 can start any time after T001.
- **Polish**: after the stories it documents. T064 can start once T015 builds, and must land before any deploy, the MVP included. T072–T074 strictly last, in order.

### Story completion order

```
Setup → Foundational → US1 (MVP) → US2 ─┬→ US5
                           ├→ US3 ──────┘ (history for replaced uploads)
                           └→ US4
                                     → Polish → full CI → docker IT → screenshots
```

### Within each story

Tests first and failing → helper op → server dispatch → UI → story Playwright green → commit.

---

## Parallel Execution Examples

### Foundational fan-out (four agents, `CARGO_BUILD_JOBS=4`, no CI)

```text
Agent A (core):      T006, T007, T008, T009, T010, T011, then T058
Agent B (helper):    T013 → T014 → T015
Agent C (adapter):   T012, T016 → T017 → T018 → T019
Agent D (web):       T004, T005, then T026–T028 once T020 bindings land
Main session:        T001–T003, T020–T025, merges
```

### User Story 1

```text
T029, T030, T031 together (tests)
T034, T035, T036 together (UI pieces) while T032 → T033 run on the server side
```

### After US1, three tracks

```text
Track 1: US2 (T039–T045) then US5 (T058–T063)
Track 2: US3 (T046–T051)
Track 3: US4 (T052–T057)
```

Tracks 2 and 3 both edit apps/depths/volume-helper/src/ops.rs, apps/depths/server/src/files.rs and
Listing.tsx: give each its own worktree and merge one at a time, or split ops.rs into
`ops/{read,write,transfer,manage}.rs` first.

---

## Implementation Strategy

### MVP first (US1)

1. Setup + Foundational. Stop: run the escape suite and the gate tests.
2. US1. Stop: Playwright `us6-files` browse spec green. This ships as a read-only explorer.
3. Optional deploy to steam, only after asking. It needs T064 (the helper in the image), which can be pulled forward right after T015.

### Incremental delivery

1. + US2: the requested feature (edit config safely). Deploy candidate, after asking.
2. + US3: backups and restores of world saves.
3. + US4 and US5: housekeeping and the undo net.
4. Polish, one full CI, the opt-in docker test, screenshots last.

### Notes

- The helper is the only code that opens volume paths. A server task that seems to need a path on
  the host is a design error: stop and check research.md R1.
- Any change to the helper argv must update contracts/helper.md and the T017 argv test together.
