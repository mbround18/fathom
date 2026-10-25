# Contract: configuration (054)

## Depths server (env / flags)

| Setting | Env | Default | Notes |
|---|---|---|---|
| Runtime endpoint | `DOCKER_HOST` | `tcp://guard:2375` in compose | Unchanged mechanism; the compose default now points at the guard. |
| Helper image | `DEPTHS_HELPER_IMAGE` | `depths-volume-helper:dev` in compose | **Required.** Resolved to an image id at start. The image must carry `org.fathom.depths.helper.build=<DEPTHS_BUILD_ID>`; otherwise files are `helper_unavailable`. There is no fallback to Depths' own image. |
| Step-up window | `DEPTHS_STEP_UP_WINDOW` | `300` (seconds) | 30–3600. Values outside that range clamp, with a startup warning. |
| Guard address | `DEPTHS_GUARD_URL` | derived from `DOCKER_HOST` | Used for `/_depths_guard/v1/*` reads. |
| Public URL | `DEPTHS_PUBLIC_URL` | (existing) | Supplies the passkey relying-party id (its host) and origin. Changing the host makes existing passkeys stop working, and the guide says so. |

Build-time: the builder sets `DEPTHS_BUILD_ID` to the SHA-256 of the helper binary. Both the
server and the helper get it through `option_env!`. Local development builds without it use `dev`.

## Guard

The guard's config is `guard.toml` (data-model.md). The example compose ships
`apps/depths/guard.toml.example` with `stacks_dir` set and no exceptions.

| Env | Default | Notes |
|---|---|---|
| `DEPTHS_GUARD_CONFIG` | `/etc/depths-guard/guard.toml` | |
| `DEPTHS_GUARD_LOG_DIR` | `/var/log/depths-guard` | |
| `RUST_LOG` | `info` | |

## Compose env (`apps/depths/.env.example`)

New:

- `DEPTHS_GUARD_IMAGE`
- `DEPTHS_HELPER_IMAGE`
- `DEPTHS_STEP_UP_WINDOW`
- `GUARD_LOG_DIR` (default `./guard-log`)

Every service keeps `restart: ${RESTART_POLICY:-no}`.
