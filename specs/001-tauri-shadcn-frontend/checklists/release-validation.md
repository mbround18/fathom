# Release validation (T126): Linux

**Scope**: the MVP ships Linux only, unsigned (deb, rpm, AppImage). This file records the Linux
part of T126: package install checks and first-run behaviour. Windows 11 and macOS 26 runs come
after the MVP and will be added here.

**Run**: 2026-09-26, version `0.1.0`, x86_64, built locally with
`CARGO_BUILD_JOBS=4 pnpm -C ui tauri build` followed by `scripts/appimage-host-libs.sh` on the
AppImage. aarch64 bundles were not built or tested in this run.

**Method**: each package installed into a fresh, throwaway `ubuntu:26.04` or `fedora:latest`
(Fedora 44) container. The app ran under Xvfb with a kubeconfig built for the run: synthetic
files, or one written by `kind create cluster --kubeconfig <file>` for a disposable
`fathom-pkgtest` cluster. No shared or production kubeconfig was read.

## Package install

| Check | deb / Ubuntu 26.04 | rpm / Fedora 44 | AppImage / Ubuntu 26.04 | AppImage / Fedora 44 |
|---|---|---|---|---|
| Installs, dependencies resolve | ✅ `apt install` | ✅ `dnf install` | n/a (runs in place) | n/a |
| Declared dependencies | ✅ `ca-certificates, libwebkit2gtk-4.1-0, libgtk-3-0` | ✅ `ca-certificates`, `libwebkit2gtk-4.1.so.0`, `libgtk-3.so.0` | n/a | n/a |
| `ldd` shows no `not found` | ✅ | ✅ | ✅ (with bundled lib path) | ✅ |
| `fathom-desktop --version` | ✅ `fathom 0.1.0` | ✅ | ✅ | ✅ |
| `.desktop` entry: `desktop-file-validate`, `Name=Fathom`, `Categories=Development;`, `Comment` | ✅ | ✅ | ✅ (embedded entry) | ✅ |
| hicolor icons 32, 64, 128, 256, 512 (and 256@2), all in `index.theme` | ✅ | ✅ | ✅ (`.DirIcon`) | ✅ |
| Headless start (Xvfb): process stays up, no panic, no missing library | ✅ | ✅ | ✅ | ✅ |
| Window renders (screenshot is not blank) | ✅ | ✅ | ✅ | ✅ |
| No CA bundle on the host: starts without panicking | ✅ (warns once, HTTPS integrations fail until `ca-certificates` is installed) | n/a (dependency) | n/a | n/a |

### Found and fixed in this run

| Problem | Before | After |
|---|---|---|
| Empty `Categories=` in the desktop entry, so the app was missing from menus | all packages | `bundle.category = DeveloperTool`, which becomes `Categories=Development;` |
| No 256 or 512 px icon, so HiDPI launchers upscaled 128 px | all packages | `icons/256x256.png` and `icons/512x512.png` added to `bundle.icon` |
| No publisher, homepage, license, short or long description | deb/rpm metadata blank | filled in `tauri.conf.json` |
| Backend panic at start on a host with no CA bundle (`reqwest` client `build().unwrap()`) | crash | one shared client (`src/backend/http_client.rs`) falls back to an empty root store and logs a warning; the packages also depend on `ca-certificates` |
| AppImage: `Could not create default EGL display: EGL_BAD_PARAMETER. Aborting...`, blank window | Ubuntu 26.04 and Fedora 44 | the bundled `libwayland-client.so.0` shadowed the host copy that Mesa's EGL driver links. Removing that one file fixes it on both distros (removing each of the other 8 bundled Mesa dependencies alone did not). `scripts/appimage-host-libs.sh` repacks the AppImage without it. |

**Open (release pipeline)**: `release.yml` uses `tauri-action`, which does not run
`scripts/appimage-host-libs.sh`. Until a step running it after the bundle is added, a tagged
release would ship an AppImage that shows a blank window on current Mesa. The Tauri bundler cannot
pass `--exclude-library` to linuxdeploy, so the post-build repack is needed.

## First run and failure states

Each scenario was run against the installed deb under Xvfb and screenshotted after 14 to 20 s. The
same states are covered in the mock e2e suite (`ui/tests/e2e/first-run-states.spec.ts`) and in
Rust unit tests (`src/backend/first_run.rs`).

| # | Scenario | Before | After |
|---|---|---|---|
| 1a | `KUBECONFIG` points at a missing file | "No kubeconfig" pane that printed the whole raw `failed to infer config` chain, and the same error in red in the status bar | **No kubeconfig found**: names the path searched. Actions: **Reload kubeconfig** and **Create a local cluster** (opens the kind panel in Cluster Settings). Raw error hidden, no error toast. |
| 1b | `KUBECONFIG` unset, empty `HOME` | same as 1a | same as 1a, naming `~/.kube/config` |
| 2 | Empty kubeconfig file | "No kubeconfig" pane with the raw `failed to determine current context` chain; red error in the status bar | **No clusters configured**: reload or create a local cluster |
| 3 | Malformed YAML | "No kubeconfig" pane with the parse error repeated four times in one raw chain | **Kubeconfig could not be parsed**: file path, line and column. Reload or create a local cluster, verbatim error under **Details** |
| 3b | Valid kubeconfig, no `current-context` | Pods pane stuck on a loading skeleton, "No kubeconfig" in the Logs tool, raw error in the status bar | **Pick a context**: one button per context, which then connects |
| 4 | Server unreachable (`https://127.0.0.1:1`) | context reported **Connected** (the client is lazy); every pane showed `ServiceError: client error (Connect)...` | **Cannot reach dead-cluster**: names the server. **Retry**, other contexts, create a local cluster, verbatim error under Details. The context dot is red. |
| 5 | Invalid token (kind API server, bogus bearer token) | reported **Connected**; every pane showed `ApiError: Unauthorized ... code: 401` | **kind-fathom-pkgtest-badtoken rejected the credentials**: explains the 401 and says to refresh credentials, with Retry |
| 6 | kind cluster deleted mid-session | pods stayed on screen as if live. The status bar cycled `Deployment watcher error (retrying in 1s)...` with one notice per watcher. | **Cannot reach kind-fathom-pkgtest** in every pane, with Retry and create-cluster actions. One status-bar notice: "Lost the connection... Fathom keeps retrying in the background". Watcher outages are deduplicated into one notice; the panes recover when the server comes back (mock e2e). |

The status bar shows the problem ("Cannot reach ...") instead of a stale "Connecting to cluster..."; that last change landed after the screenshots and is covered by unit tests.

None of the scenarios showed a blank screen, an endless spinner, a panic, a crash or a toast storm.
A kind cluster created from the first-run state becomes the active context automatically (unit and
e2e tested; not exercised live because it needs Docker inside the app container).

**Known noise**: when a cluster disappears, `kube_client` logs one `ERROR ... client error
(Connect)` line per watcher retry to stderr (222 lines in 75 s in scenario 6). The UI is not
affected.

## Not covered in this run

- aarch64 deb, rpm and AppImage (no arm64 host or emulation in this run).
- Real desktop sessions (GNOME on Wayland, KDE). All runs used Xvfb with software Mesa.
- Upgrade from an earlier package version (there is none yet).
- Windows 11 and macOS 26 (post-MVP).
