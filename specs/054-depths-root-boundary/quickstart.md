# Quickstart: validating 054

These are run by the main session, one at a time, with `CARGO_BUILD_JOBS=4`. Subagents run only
the targeted commands in §1.

## 1. Targeted tests (no Docker)

```
cargo test -p depths-guard-core      # rule table: every FR-003/FR-005 case refused, ordinary bodies allowed (SC-001)
cargo test -p depths-guard           # proxy against a fake upstream: refusals never forwarded, unknown endpoints,
                                     # upgrades, guard down → nothing forwarded (SC-003), log lines, exceptions endpoint
cargo test -p depths-server --test passkeys   # soft passkey: enrol, verify, replay/origin rejection,
                                              # copied-cookie cannot act (SC-005), step-up window, recovery
cargo test -p depths-protocol        # round trips for new variants; ts-rs bindings regenerated
pnpm --filter @fathom/depths-web test
pnpm --filter @fathom/depths-web exec playwright test --project us7-passkeys   # virtual authenticator
pnpm --filter @fathom/depths-web exec playwright test --project us8-guard      # refusal display, exceptions list
```

## 2. Guard against a real daemon (opt-in)

```
docker build -f apps/depths/Dockerfile --target guard  -t depths-guard:dev .
docker build -f apps/depths/Dockerfile --target helper -t depths-volume-helper:dev .
DEPTHS_IT_DOCKER=1 cargo test -p depths-guard --test docker
```

The test starts a socket proxy and a guard on scratch networks named `depths-it-<random>`. It then
uses the Docker CLI with `DOCKER_HOST=tcp://<guard>` to check the following, and removes
everything it created, by name:

- Each FR-003 case fails with `depths-guard refused`.
- The example stacks deploy (SC-002).
- A symlink planted under the stacks folder is refused.
- `--volumes-from <socket-proxy>` is refused.
- A container on `docker-api` is refused.
- An exception lets exactly one device through.

## 3. Helper image

```
docker image inspect depths-volume-helper:dev --format '{{.Config.Labels}}'   # has helper.build label
docker save depths-volume-helper:dev | tar -t | ...                           # one layer, one file (SC-004)
DEPTHS_IT_DOCKER=1 DEPTHS_HELPER_IMAGE=depths-volume-helper:dev cargo test -p depths-server --test files_docker
```

## 4. End to end on the dev compose

1. Write `guard.toml` from the example, then run `docker compose run --rm guard check`. It lists
   the stacks that need exceptions.
2. Run `docker compose up -d`, sign in as owner, register a passkey, and deploy the example stacks.
3. Deploy a stack that binds `/etc`. The deploy fails, the refusal appears next to the service, and
   both the audit log and `guard-log/` record it.
4. Wait out the step-up window, then open a shell. The passkey prompt comes first.
5. Stop the guard. Every action reports "runtime guard is not answering".

## 5. Before upgrading steam

Steam is the production deployment. On it:

1. Run `guard check`.
2. Write the exceptions the check suggests.
3. Have the owner enrol a passkey right after the upgrade (research R11).
