# Quickstart: validating the volume explorer (053)

Run one heavy command at a time, with `CARGO_BUILD_JOBS=4` and `nice` (repo CLAUDE.md).
Subagents never run `scripts/ci.sh`, `paws` or `dagger`.

## 1. Core and helper

```sh
CARGO_BUILD_JOBS=4 nice cargo test -p depths-volumes          # RelPath, frames, text codec, history policy
CARGO_BUILD_JOBS=4 nice cargo test -p depths-volume-helper    # escape suite (SC-003), atomic write (SC-004)
```

**Expected**:
- At least 30 escape cases pass. Each one either reads or writes inside the test root, or is
  refused as `outside_mount`/`write_blocked`.
- None of them touches the sentinel file planted outside the root.
- Writes keep uid, gid and mode, and a write killed mid-way leaves the old content.

## 2. Server

```sh
CARGO_BUILD_JOBS=4 nice cargo test -p depths-server files     # gates, audit, history, routes (fake helper)
CARGO_BUILD_JOBS=4 nice cargo test -p depths-protocol         # round trips; then regenerate bindings:
CARGO_BUILD_JOBS=4 nice cargo test -p depths-protocol --test export_bindings
```

**Expected**:
- A viewer is refused on every command and both routes.
- Under the restricted view, only `ListMounts` and `ListDir` pass.
- With `DEPTHS_FEATURE_FILES=read_only`, every change is refused `feature_read_only`.
- Each refusal writes one `files.refused` audit entry.

## 3. UI against the fake backend

```sh
cd apps/depths/web
nice pnpm test                                               # explorer logic
DEPTHS_E2E_PORT=4196 nice pnpm exec playwright test --project=us6-files
```

**Expected**:
- **US1–US5 scenarios**: the Files tab, browsing a 50,000-entry folder (SC-002 timing asserted),
  the editor with highlighting, a binary file offering download, a save conflict that offers
  reload or compare, save and restart, an upload confirm naming every file, delete with a count, and a
  history restore.
- At 360 px wide the page has no sideways scroll.

## 4. Real helper on a scratch volume (opt-in, needs Docker)

```sh
docker build -f apps/depths/Dockerfile -t depths:dev .
DEPTHS_IT_DOCKER=1 DEPTHS_HELPER_IMAGE=depths:dev CARGO_BUILD_JOBS=4 nice cargo test -p depths-server --test files_docker
```

The test creates a volume `depths-it-<random>` and a bind directory with files owned by
1000:1000 at 0640, then:
- lists them;
- saves one, and checks it is still `1000:1000 0640`;
- downloads the folder as tar;
- uploads a file;
- tries `../`, an absolute link and a link swap.

It removes the volume, the directory and any `depths.helper` containers it started, and leaves
nothing else behind. Check with `scripts/hygiene.sh`.

## 5. On a real host (after a deploy that has been asked for and approved)

1. Open a stack with a named volume (for example a Valheim stack) and choose the **Files** tab.
2. Open `/config`, then `adminlist.txt`. Add a line, save, and confirm the diff.
3. On the host, `stat -c '%u:%g %a'` shows the file's owner and mode unchanged.
4. Open the file's history and restore the previous version.
5. Look in the audit log for `files.read`, `files.save` and `files.restore`.
6. Try a sensitive bind (`/`), and check that it is listed as not browsable, with the reason.
