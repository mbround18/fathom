# Quickstart: validating Depths (050)

Run from the repository root. `CARGO_BUILD_JOBS=4` throughout. Fixtures use `busybox` only and
project names prefixed `depths-test-`; clean up by name, never by prune.

## 1. Fathom is unbroken by the extraction (FR-044, SC-007)

```bash
scripts/ci.sh                                   # fmt, clippy, tests, bindings drift, UI build
git diff --exit-code ui/src/protocol/generated  # after: cargo test -p fathom --test export_bindings
pnpm --filter fathom-ui exec playwright test    # Fathom E2E, self-sized workers
```

Expected: all green; generated bindings byte-identical to before the extraction.

## 2. Depths unit, round-trip and E2E

```bash
cargo test -p container-runtime -p web-auth -p audit-log -p depths-core -p depths-stacks-fs -p depths-protocol -p depths
cargo test -p depths-protocol --test export_bindings && git diff --exit-code apps/depths/web/src/protocol/generated
pnpm --filter @companion/depths-web test                  # Vitest
pnpm --filter @companion/depths-web exec playwright test  # all stories, fake backend
pnpm --filter @companion/depths-web exec playwright test --project=us2-lifecycle   # one story
```

## 3. Real daemon, local (no IdP needed)

The dev server takes a built-in fake OIDC issuer only under `--insecure-dev` on loopback.

```bash
S=$(mktemp -d)/stacks; mkdir -p "$S/depths-test-web" "$S/depths-test-jobs"
cat > "$S/depths-test-web/compose.yaml" <<'EOF'
services:
  app: { image: busybox, command: sleep 3600 }
EOF
cat > "$S/depths-test-jobs/compose.yaml" <<'EOF'
services:
  init: { image: busybox, command: "true" }
  app:  { image: busybox, command: sleep 3600 }
EOF
cargo run -p depths -- --insecure-dev --listen 127.0.0.1:8080 --public-url http://127.0.0.1:8080 \
  --stacks-dir "$S" --static-dir apps/depths/web/dist --owner-user-id oidc:dev-admin
```

Check, signed in as the dev owner (`dev-admin` in the dev OIDC provider):

| Step | Expected |
|---|---|
| Open the list | Both stacks, `not deployed`. |
| Up `depths-test-jobs` | Preview shows `docker compose up -d --remove-orphans`, cwd = its dir. After confirm: `running`, **not** partial, `init` shown as completed (R2). |
| Edit `depths-test-web` in a terminal while its editor is open, then save in Depths | `Conflict` with the new text (FR-019). |
| Save with a broken indent | Line-anchored error; file unchanged (`sha256sum` before/after). |
| Open logs + shell + stats for both `app` containers in tabs, reload | Layout restored; shells reconnect as new sessions. |
| Sign in as `dev-user` | No-access page; the owner sees `oidc:dev-user` as pending. |
| As the owner, make `dev-user` a user on `depths-test-web` only; act as `dev-user` | Only `depths-test-web` listed, no foreign or standalone group; a forged `Confirm` for `depths-test-jobs` over the socket returns `Denied`, audited. |
| Down both, then `docker compose ls --all` | No `depths-test-*` projects remain. |

Then `rm -rf "$S"`.

## 4. Dockge round trip (SC-002)

Point Dockge 1.5.0 and Depths (in turn) at the same copy of a stacks dir; `find "$S" -type f -exec sha256sum {} + | sort` is identical before and after a session in Depths that only ran lifecycle actions.

## 5. Container + socket proxy (US6, R8)

```bash
docker build -f apps/depths/Dockerfile -t depths:dev .
cd apps/depths && STACKS_DIR="$S" docker compose up -d   # RESTART_POLICY unset → no restart
```

| Check | Expected |
|---|---|
| `docker compose ps` | Both services up; `depths` runs as uid 10001, no socket mounted in it. |
| Open a shell tab through the proxy, idle 11 min | Tab shows "reconnected"; typing works. |
| Remove the provider env and restart | Depths refuses to start: no provider configured. |
| `curl -s localhost:8080/api/v1/me` without cookie | 401. |

`docker compose down` in `apps/depths`, then `docker builder prune --filter until=24h`, then
`scripts/hygiene.sh`.
