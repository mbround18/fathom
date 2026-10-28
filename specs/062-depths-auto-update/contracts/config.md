# Contract: configuration (062)

Flags and environment, resolved by `config.rs`'s `Resolver::pick` like every other setting (flag
beats environment beats default). Everything an administrator changes at run time lives in
`updates.json` (data-model.md), not here.

| Flag | Environment | Default | Meaning |
|---|---|---|---|
| `--updates` | `DEPTHS_UPDATES` | `on` | `off` removes the feature: no Updates page, no scheduler task, no registry client. Kill switch (research R10). |
| `--updates-auto` | `DEPTHS_UPDATES_AUTO` | `on` | `off` pauses every Auto policy host-wide without changing it. |
| — | `DOCKER_CONFIG` | `$HOME/.docker` (`/var/lib/depths/.docker`) | Where registry credentials are read from, by the check and by the `docker` CLI alike (research R1). |
| `--updates-check-min-gap` | `DEPTHS_UPDATES_CHECK_GAP` | `60s` | Minimum time between two Check now runs (FR-002). Tests set it lower. |

Not configurable on purpose: the hold rules, the trust and passkey gates, the one-op-per-stack
lock, and that Depths never updates its own container.

## Compose (documented in `apps/depths/compose.yaml` and the guide)

```yaml
  depths:
    volumes:
      # Host local time for maintenance windows. Without it windows run in UTC.
      - /etc/localtime:/etc/localtime:ro
      # Optional: registry credentials for private images, read-only.
      # - ~/.docker/config.json:/run/depths/docker/config.json:ro
    environment:
      # DOCKER_CONFIG: /run/depths/docker
```

- The socket proxy needs nothing new: pulls, tags, inspects and removals are `/images/*`
  (`IMAGES=1`, `POST=1`), already enabled. `DISTRIBUTION` stays off.
- A `config.json` that uses `credsStore`/`credHelpers` cannot be used as-is (no helper binaries in
  the image). The guide shows `docker login --password-stdin` into a separate `DOCKER_CONFIG`
  directory to produce a plain one.

## Startup log

One line at start (JSON log, never values from `config.json`):

`updates: feature=on auto=on checks=off zone=Europe/Berlin docker_config=/run/depths/docker (2 registries)`
