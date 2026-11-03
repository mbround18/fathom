# Contributing

This repository is the orchestration companion suite: **Fathom** (Kubernetes, desktop and hosted)
and **Depths** (Docker Compose hosts, one container). This file is for people working on the code.
What users need to learn lives in [docs/guides](docs/guides/README.md); the
[README](README.md) only introduces the two apps.

PRs welcome. Before you submit, run `cargo fmt`, `cargo clippy` and the tests for what you
touched (below). Commits are signed.

## Workspace layout

One Cargo workspace (`Cargo.toml`) and one pnpm workspace (`pnpm-workspace.yaml`, one
`pnpm-lock.yaml`) cover both apps.

| Path | What lives there |
|---|---|
| `apps/depths/server` | `depths-server`, the Depths binary (`depths`): Axum + Tower, config, HTTP routes, the WebSocket, wiring |
| `apps/depths/web` | `@companion/depths-web`, the Depths React + Vite SPA |
| `apps/depths/Dockerfile`, `compose.yaml`, `.env.example` | The image and the example deployment |
| `crates/container-runtime` | Docker and Compose runtime shared by the suite: CLI runner, argv previews, streams, wire types |
| `crates/web-auth` | Browser sign-in for the suite's servers: OIDC, GitHub, Discord, sessions, CSRF |
| `crates/audit-log` | Append-only JSON-lines audit log with size rotation |
| `crates/oci-registry` | Adapter: the HEAD-first OCI registry client Keel and Depths use to spot newer images. Feature `test-util` adds an in-process fake registry |
| `crates/depths-core` | Depths' domain: stacks, status, roles, operation planning. No I/O |
| `crates/depths-stacks-fs` | Adapter: the Dockge-compatible stacks directory on disk |
| `crates/depths-runtime-cli` | Adapter: the `docker` CLI, argv only, never a shell |
| `crates/depths-protocol` | Depths' versioned command/event protocol, with generated TypeScript |
| `packages/theme` | `@companion/theme`: the neon-noir design tokens and fonts |
| `packages/ui` | `@companion/ui`: shared React components (primitives, container tables, argv preview) |
| `packages/terminal` | `@companion/terminal`: the xterm.js terminal and log view |
| `src/`, `ui/`, `ui/src-tauri/` | Fathom: the backend library, the React UI, the Tauri desktop shell |
| `server/`, `daemon/` | Fathom's hosted web server and its in-cluster aggregator |

The rule for new code: `apps/*` holds entry points only, thin layers that wire libraries together.
Logic goes in `crates/*` (Rust) or `packages/*` (TypeScript), so an app can be replaced without
rewriting what it uses.

**Why Fathom's roots stay where they are.** `src/`, `server/`, `ui/` and `daemon/` predate this
layout. Moving them into `apps/` would touch every Tauri, release and CI path and give Depths
nothing, so they stay put; that move is a separate change if it is ever made. What Fathom shares
with Depths has been extracted into `crates/` and `packages/` instead, and Fathom depends on those.

### Depths' shape

`depths-core` is a hexagonal core: plain types and decisions, no I/O. It talks to the outside
through two ports, `StackStore` (implemented by `depths-stacks-fs`) and `ContainerRuntime`
(implemented over `container-runtime`). The server in `apps/depths/server` owns the adapters and
the transport. Every command Depths runs is an argv handed to a runner, never a host shell;
`crates/depths-core/tests/no_shell.rs` scans the Depths sources to keep it that way.

The browser and the server speak one versioned protocol over one WebSocket (`/api/v1/ws`). Its
types are defined once in Rust in `depths-protocol` and exported to TypeScript with ts-rs into
`apps/depths/web/src/protocol/generated/`. Do not edit the generated files.

### Fathom's shape

The `fathom` library crate in `src/` is the backend: every Kubernetes call, integration and the
MCP server run there on a tokio runtime, behind a typed command/event protocol (`src/protocol/`,
exported to TypeScript as `ui/src/protocol/generated/`). `ui/src-tauri` is the Tauri v2 desktop
shell that hosts the React + TypeScript UI in `ui/` and bridges it to the backend. `server/`
(`fathom-server`) serves the same UI and backend as an in-cluster web app with OIDC sign-in, and
`daemon/` (`fathom-daemon`) is the optional in-cluster aggregator for topology, metrics and event
snapshots. Everything the UI shows comes through the protocol, so the shells are thin.

---

## Shared-crate rules

The shared crates are shared because they know nothing about either app.

- `container-runtime` must not depend on `kube`, `tauri` or any `fathom` crate.
- `web-auth` and `audit-log` must not depend on `kube` or any `fathom` crate.
- `oci-registry` holds no Keel or Depths types and depends on no app crate. Each app maps its
  results into its own model and names its own metrics. It may make only the registry requests
  listed in `specs/061-keel-update-notice/contracts/registry.md`: `HEAD` to learn a tag's
  digest, and a manifest `GET` (which registries count as a pull) only by digest, for an image
  the caller hasn't seen.
- App-specific behaviour goes behind a trait the app implements (Fathom's cluster impact is a
  `ClusterImpact` that Depths passes as `None`), not behind a dependency on the app.

`scripts/ci.sh shared` enforces these with `cargo tree`. To check by hand:

```sh
cargo tree -p container-runtime --prefix none | grep -E '^(kube|tauri|fathom)([- ]|$)'
cargo tree -p web-auth -p audit-log --prefix none --format '{p}' | grep -E '^(kube|fathom)([- ]|$)'
```

Both should print nothing.

**Bindings drift.** Generated TypeScript must match the Rust it comes from. After changing a
protocol type, regenerate and commit the result:

```sh
cargo test -p fathom --test export_bindings            # ui/src/protocol/generated/
cargo test -p depths-protocol --test export_bindings   # apps/depths/web/src/protocol/generated/
```

`scripts/ci.sh fathom` and `scripts/ci.sh depths` regenerate their bindings and fail if the
directory changed. A change to a shared
crate must leave Fathom's bindings byte-identical.

## Shared UI kit

`@companion/ui/kit` (`packages/ui/src/kit`) holds the screen-level pieces Keel, Depths and Fathom
share: states, dialogs, headers, tables, navigation. Build a new screen from these, and import
them from `@companion/ui/kit`, never from a deep path. The kit holds no app words or rules: the
text, the refusal, the guides and the glossary come from the app. Spec 065 has the details
(`specs/065-shared-ui-kit/contracts/kit-api.md`).

### The pieces

| Piece | Use it when |
|---|---|
| `KitProvider`, `useKit` | Once at the app root, to hand the kit your link, guides and glossary (below) |
| `navigate`, `usePath`, `useSearchParam`, `useHash`, `matchPath`, `setNavigationGuard` | Routing inside the app; the in-app event is `companion:navigate` |
| `Link` | Any link to another screen of the same app |
| `Refusal`, `isTransportFailure`, `ListHandlers`, `ListLoad` | Typing what the server refused and how a list loads; `OFFLINE`, `refusalOf` and `loadList` stay in the app |
| `RefusalText` | Showing a refusal ("not allowed" or an error) inline, as the server sent it |
| `LoadFailed` | A list or panel that failed to load: "Failed to load {what}." with the reason and Retry |
| `LoadState` | Switching a list between loading, failed, empty and loaded |
| `ErrorLine` | A dismissible one-line error under a form or toolbar |
| `Page` | The outer frame of a routed page |
| `PageHeader` | A page's title, intro, guide link and actions |
| `PageErrorBoundary` | Around a page or pane, so a crash shows "Something went wrong" there and nowhere else |
| `EmptyState`, `emptyVariant` | A list with nothing in it yet: why it matters, the next steps, and the one action (or who can take it) |
| `NoMatch` | A filter or search that matched nothing, with a way to clear it |
| `Dialog` | A modal that is not a confirmation |
| `ConfirmDialog`, `PreviewDetails` | Confirming a change the server previewed: its summary, what it affects, and whether it asks for a passkey |
| `CopyBlock`, `copyText` | A value someone will copy (a token, a command), masked when secret |
| `useReveal` | Remembering which secret values are shown |
| `Field`, `Select`, `FormPanel`, `SectionIntro`, `FIELD`, `LABEL`, `H1`–`H3`, … | Forms and section headings, with the suite's sizes and classes |
| `Tabs` | Tabs inside a page; controlled, and only the active panel renders |
| `DataTable` | A short list: a table at 640 px and wider, cards below |
| `Markdown` | Rendering guide or help text; load it with `React.lazy`, since it pulls in react-markdown |
| `slug`, `slugger` | Heading anchors that match the guides' |
| `GuideLink` | A "Learn more" link to a guide; renders nothing without `guideHref` |
| `TermTip` | A term with its glossary entry in a popover; renders nothing for a term the glossary lacks |
| `NavShell` | The app frame: header, banner, grouped side navigation (a sheet below 768 px) and the main area |

Coming in this spec, being added now:

- `DataGrid`: a long or live list (thousands of rows). It is virtualized and keyboard-driven, and
  hides columns by priority as the width shrinks. It is fully controlled: rows, sort, selection
  and the focused row come in as props. Fathom's resource table is built on it.
- `DataTable` and `DataGrid` gain optional `sort` (`{ value, onChange }`; a column sorts only with
  `sortable: true`, and a click cycles ascending, descending, none), `selection` (a set of
  `rowKey(row)` values) and `rowActions(row)` (a trailing menu). Without them a table renders as
  today.
- `PageHeader size="pane"`: a compact one-row header for a pane, with `count`, `lead` and
  `actions` slots, and `rootProps` for the pane's own attributes. `size="page"` is the default.
- `ConfirmDialog typedConfirm={{ word }}`: Confirm stays disabled until the word is typed. Use it
  for deleting many items at once.
- `PageErrorBoundary resetKey`: when the key changes (another pane, another resource), a crashed
  boundary renders its children again.

### Examples

A list with nothing in it:

```tsx
<EmptyState
  page="widgets"
  title="No widgets yet"
  reason="Widgets show what each host runs."
  steps={['Add a host.', 'Pick the widgets to watch.']}
  action={canAdd ? <Button onClick={add}>Add widget</Button> : undefined}
  whoCan="An owner or admin"
  verb="add widgets"
  guide={{ guide: 'widgets', anchor: 'add' }}
/>
```

A list that failed to load:

```tsx
{refusal ? <LoadFailed what="widgets" refusal={refusal} onRetry={load} /> : <WidgetList rows={rows} />}
```

Confirming a change the server previewed:

```tsx
<ConfirmDialog
  preview={preview}            // null keeps it closed
  title="Remove widgets"
  confirmLabel="Remove"
  tone="destructive"
  onConfirm={() => void confirm()}
  onCancel={cancel}
/>
```

### `KitProvider`

Each app wraps its root once: `<KitProvider value={{ … }}>`. Every field is optional, and a piece
whose field is missing renders without it.

| Field | What it is | Without it |
|---|---|---|
| `Link` | The app's in-app link component (`{ to, …anchor props }`) | The kit's own `Link` |
| `guideHref` | Builds a guide's route from `{ guide, anchor? }` | `GuideLink` renders nothing; Markdown keeps guide links as plain links |
| `isGuide` | Tells whether a Markdown link target (`slug.md`) is one of the app's guides | No link is treated as a guide |
| `guideImage` | The bundled URL of a guide's image (`img/<name>`) | The image shows as a link |
| `glossary` | The app's terms: `{ [id]: { term, text, guide? } }` | `TermTip` renders nothing |

### Testing a piece

Each piece has unit tests (`packages/ui/tests/unit`, Vitest) and an isolated browser spec
(`packages/ui/tests/e2e/<piece>.spec.ts`) that runs against the harness, a page that shows one
piece alone, with no app or server.

```sh
pnpm --filter @companion/ui test                                    # unit tests
pnpm --filter @companion/ui harness                                 # the harness in a browser
pnpm --filter @companion/ui e2e tests/e2e/empty-state.spec.ts       # one piece's spec
pnpm --filter @companion/ui e2e                                     # every piece
```

The harness URL is `/?piece=<id>&state=<state>&theme=<light|dark>`. Without `piece` it lists every
piece, and without `state` it lists the piece's states. A piece's fixture is
`packages/ui/harness/pieces/<id>.tsx`, and its content never names an app.

Every spec calls `expectAccessible` (`packages/ui/tests/support/a11y.ts`) on every state: axe with
the WCAG 2.2 AA tags, in light and dark, at 360 and 1280 px, with zero violations. No rule is
turned off for the whole kit. To turn one off for one piece, pass it in `disable` and write the
reason in a comment beside it in that piece's spec.

### No copies in the apps

`node scripts/check-kit-copies.mjs` (part of `pnpm -w run lint:packages`) fails when Keel, Depths
or Fathom (`apps/keel/web/src`, `apps/depths/web/src`, `ui/src`) declares, at the top level of a
file, something with the name of a kit export. It names the file and the line, and tells you to
import the piece from `@companion/ui/kit`. Imports, usages, nested helpers and test files don't
count, and nor does an adapter that imports the kit piece under another name and wraps it
(`import { ErrorLine as Alert }`, then `function ErrorLine()` around it).

When an app must keep its own piece under a kit name, add it to `packages/ui/kit-allow.json`:

```json
[{ "file": "ui/src/components/Foo.tsx", "name": "EmptyState", "reason": "why it can't use the kit's" }]
```

`file` is relative to the repository root. An entry without a `reason`, or one that no longer
matches anything, fails the check, so keep the list short and remove entries when the copy goes.

## Fathom development

Commands run from the repo root (full detail in `specs/001-tauri-shadcn-frontend/quickstart.md`).

```bash
pnpm install                           # the root workspace: ui, packages/*, apps/depths/web
cargo test -p fathom export_bindings   # generates ui/src/protocol/generated/

# Run
pnpm -C ui tauri dev                   # full app: Vite HMR + Rust backend
pnpm -C ui dev:fake                    # browser only, fake transport (no cluster needed)
cargo run -p fathom-desktop -- --only-mcp   # headless MCP server

# Test
pnpm -C ui lint && pnpm -C ui typecheck
pnpm -C ui test                        # Vitest
pnpm -C ui e2e                         # Playwright: chromium + webkit x phone..ultra
cargo test --workspace                 # backend, serde round-trips, bindings drift

# Rust checks
cargo check
cargo clippy
cargo fmt

# CI, exactly as GitHub runs it (containers via paws + Docker)
scripts/ci.sh                          # the slices your diff touches (changed)
scripts/ci.sh keel                     # one slice: shared|fathom|fathom-full|depths|keel
scripts/ci.sh validate                 # what your diff touches, without Playwright
scripts/ci.sh all                      # every slice, Fathom e2e at three widths
CI_E2E=full scripts/ci.sh all          # every slice, full e2e matrix: the release gate

# While iterating: the same slices on the host, minus Playwright, web builds and containers
make gate                              # GATE_SLICES="shared keel" to pick, GATE_BASE to diff against
```

Every slice reports its steps slowest first, the time the engine spent outside them, and the
change against its last pass, and appends a line to `~/.local/state/fathom/ci-times.tsv`
(`CI_TIMES`). Look at the trend with `column -t -s$'\t' ~/.local/state/fathom/ci-times.tsv | tail`
and treat a slice that grows as a bug: the slowest step is where to cut first.

`scripts/ci.sh` is the only thing `.github/workflows/ci.yml` runs, so a green local run is a green
CI run. CI is sliced per app: `changed`, the default, diffs against `origin/main` (`CI_BASE`) and
runs only the slices that touch, so a Keel change gates Keel's crates, bindings and web app in
minutes. Docs and specs run nothing; a shared crate or a lockfile runs `shared` and every app that
uses it. The `fathom` slice runs its e2e on chromium at phone, laptop and wide; `fathom-full` runs
the whole 7-width chromium and webkit matrix, over an hour, before a release. `CI_E2E` picks the
e2e for every slice: `quick` (the default), `full` (the matrix for `fathom` and `all` too) or
`none` (no Playwright at all: lint, typecheck, unit tests, clippy, cargo tests and the binding
drift checks only). `validate` is `changed` with `CI_E2E=none`. It needs
[`paws`](https://github.com/mbround18/paws) with `paws run`
(`cargo install --locked --git https://github.com/mbround18/paws --branch feat/fathom-local-ci paws-cli`
until that is released) and Docker. It builds from your working tree minus everything `.gitignore`d,
so uncommitted edits are tested but local `target/` and `node_modules/` are not. It is sized for a
shared workstation: `CARGO_BUILD_JOBS=4`, `PLAYWRIGHT_WORKERS=2`, niced, and the `fathom` slices refuse to
start with less than 8GB of memory available (`CI_MIN_FREE_GB`). Build caches live in the dagger
engine's `fathom-ci-*` cache volumes.

Performance checks: `scripts/measure-idle-cpu.sh` (SC-005) and `scripts/measure-startup.sh` (SC-003).

## Depths development

Prerequisites: the Rust toolchain from `rust-toolchain.toml`, Node 22+ with pnpm, and Docker with
the Compose plugin (Depths runs the `docker` CLI).

```sh
pnpm install
make depths-dev
```

`make depths-dev` runs two things together and stops both on Ctrl-C:

- the server, `cargo run -p depths-server`, with `DEPTHS_LISTEN=127.0.0.1:8080`,
  `DEPTHS_PUBLIC_URL=http://127.0.0.1:5174` and `DEPTHS_INSECURE_DEV=true`;
- Vite on `http://127.0.0.1:5174`, which proxies `/api` (with WebSockets) and `/auth` to the server.

Open `http://127.0.0.1:5174`. `--insecure-dev` is what allows the `http://` public URL and a
cookie without the `__Host-` prefix; the server refuses it on any non-loopback listen address.

The target sets only those variables. The server also needs, from your environment:

- **A sign-in provider.** The server has no built-in fake provider and no anonymous mode, so
  without one it refuses to start. The quickest is a GitHub OAuth App (or a Discord application)
  of your own for development, with the callback `http://127.0.0.1:5174/auth/callback/github` (or
  `.../discord`).
- **An owner**, so you are admin on first sign-in: `OWNER_USER_ID=github:<your login>`.
- **A stacks directory and a data directory** you can write. The defaults, `/opt/stacks` and
  `/var/lib/depths`, are the container's.

For example:

```sh
mkdir -p ~/depths-dev/stacks ~/depths-dev/data
export DEPTHS_STACKS_DIR=~/depths-dev/stacks DEPTHS_DATA_DIR=~/depths-dev/data
export OAUTH_PROVIDER=github OAUTH_CLIENT_ID=<id> OAUTH_CLIENT_SECRET=<secret>
export OWNER_USER_ID=github:<your login>
make depths-dev
```

To work on the UI without a server or a provider, use the Vitest tests or one Playwright story
against the fake backend (below).

**The image.** `make depths-image` builds `depths:dev` from `apps/depths/Dockerfile`: cargo-chef
stages for the server, a pnpm stage for the web app, the Docker CLI and Compose plugin copied in,
and a `debian:trixie-slim` runtime. The web assets are files under `STATIC_DIR`, never embedded in
the binary. `apps/depths/compose.yaml` with a `.env` made from `.env.example` runs it.

## Test layers

| Layer | What | Run |
|---|---|---|
| Unit (Rust) | Each crate's own tests: config resolution, planning, the fs adapter, the CLI adapter against stand-in binaries (no Docker daemon needed) | `cargo test -p <crate>` |
| Protocol | serde round-trips and the bindings export | `cargo test -p depths-protocol` |
| Server | The router and sign-in in-process with a fake provider (`tests/http.rs`); the WebSocket over real TCP (`tests/ws.rs`) | `cargo test -p depths-server` |
| Unit (web) | Vitest for the Depths web app | `pnpm --filter @companion/depths-web test` |
| End to end | Playwright against `tests/support/fake-backend.ts`, one project per user story: `shell`, `us1-stacks`, `us2-lifecycle`, `us3-edit`, `us4-tabs`, `us5-access` | `pnpm --filter @companion/depths-web e2e --project us1-stacks` |
| Static | Lint and types | `pnpm --filter @companion/depths-web lint` and `typecheck` |
| Supply chain | `cargo deny` (`deny.toml`) and `pnpm audit` of production dependencies | `make audit` |
| Everything | The full gate, as CI runs it | `scripts/ci.sh` |

`make depths-test` runs the Rust tests of the Depths and shared crates (`DEPTHS_PKGS` in the
Makefile) plus the web Vitest suite. Keep tests deterministic: no wall clock, network or Docker
daemon in unit tests.

Run one Playwright story at a time while you work. The e2e server port is `DEPTHS_E2E_PORT`
(default 4183).

## Resource rules

This is often built on a shared workstation, so the tooling is sized to share it.

- **One CI at a time.** `scripts/ci.sh` takes a machine-wide lock
  (`$XDG_RUNTIME_DIR/fathom-ci.lock`); a second run waits, or exits 75 with `CI_LOCK_WAIT=0`. Do
  not work around it. If a run is stale, stop the session that owns it (named in
  `fathom-ci.lock.owner`), not the lock. While developing, run the targeted tests above, and run
  the full gate once on the combined branch.
- **Cargo**: use `CARGO_BUILD_JOBS=4` when other builds are running.
- **Playwright**: both Playwright configs size their workers from the load average and available
  memory, and print what they picked. Set `PLAYWRIGHT_WORKERS` to pin it. Never run two full
  suites, or a full suite beside a CI run: a full suite at default parallelism uses about 12 GB.
- **Build output**: `make gc` sweeps old cargo output (`cargo sweep`, window `GC_DAYS`) and caps
  the Docker build cache (`GC_DOCKER_KEEP`); `make depths-image` runs it first. It never fails a
  build: without `cargo-sweep` it prints the install command and carries on. It never prunes
  volumes or containers.
- **Clean up**: delete the kind clusters, worktrees and images you made for a task.
  `scripts/hygiene.sh` reports what is dangling (exit 1 when something is), and
  `scripts/hygiene.sh clean` removes only what is provably safe.

## Licences

Everything here is Apache-2.0 (`LICENSE`), and every crate and package says so in its `license`
field. `THIRD_PARTY_LICENSES.md` lists the licences of the Rust dependencies and the production
npm dependencies of `ui/`, `apps/depths/web` and `packages/*`. Regenerate it after changing
dependencies:

```sh
scripts/third-party-licenses.sh
```

## Cutting a release

1. Bump the version in `Cargo.toml`, `ui/package.json` and `ui/src-tauri/tauri.conf.json`, and
   finish the `## [x.y.z]` section in `CHANGELOG.md` (its text becomes the top of the release
   notes). Commit (signed).
2. Tag and push: `git tag -s vX.Y.Z -m vX.Y.Z && git push origin refs/tags/vX.Y.Z`. A `-rc.N`
   suffix makes it a pre-release.
3. `.github/workflows/release.yml` runs on the tag: one tauri-action build per platform uploads
   bundles to a draft release, the AppImage is repacked against host libwayland, every bundle gets
   a Sigstore provenance attestation, then the publish job regenerates `SHA256SUMS`, renders the
   notes and flips the draft.
4. If Actions did not fire (it has been disabled before), run it by hand on the existing tag:
   `gh workflow run Release --ref main -f tag=vX.Y.Z`.

### Release notes

Notes are rendered from `.github/RELEASE_TEMPLATE.md` by `scripts/release-notes.sh`, which fills
in the CHANGELOG section for the version, a table of the assets on the release, the verification
instructions and GitHub's auto-generated "what's changed". Edit the template to change every
future release; to redo the notes on an existing one:

```sh
make release-notes TAG=vX.Y.Z           # preview to stdout
make release-notes TAG=vX.Y.Z APPLY=1   # write to the release
```

### Building without Actions

`make build-linux`, `scripts/appimage-host-libs.sh dist/*.AppImage`, `make stage`, then
`gh release create vX.Y.Z --verify-tag dist/*` (add `--prerelease` for an rc). Assets uploaded by
hand have a checksum but no attestation, so prefer re-running the workflow when it is available.
`make build-windows-paws` cross-builds the Windows installer in a container.
