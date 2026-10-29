# Contract: configuration (062)

Flags and environment, resolved by `config.rs`'s `Resolver::pick` like every other setting (flag
beats environment beats default). Everything an administrator changes at run time lives in
`updates.json` (data-model.md), not here.

| Flag | Environment | Default | Meaning |
|---|---|---|---|
| `--updates` | `DEPTHS_UPDATES` | `on` | `off` removes the feature: no Updates page, no scheduler task, no registry client. Kill switch (research R10). |
| `--updates-auto` | `DEPTHS_UPDATES_AUTO` | `on` | `off` pauses every Auto policy host-wide without changing it. |
| — | `DOCKER_CONFIG` | `$HOME/.docker` (`/var/lib/depths/.docker`) | Where registry credentials are read from, by the check and by the `docker` CLI alike (research R1, contracts/registry.md). |
| `--updates-check-min-gap` | `DEPTHS_UPDATES_CHECK_GAP` | `60s` | Minimum time between two Check now runs (FR-002). Tests set it lower. |

Not configurable on purpose: the hold rules, the trust and passkey gates, the one-op-per-stack
lock, and that Depths never updates its own container.

## Compose (documented in `apps/depths/compose.yaml` and the guide)

```yaml
  depths:
    volumes:
      # Host local time for maintenance windows. Without it windows run in UTC.
      - /etc/localtime:/etc/localtime:ro
      # Optional: registry credentials for private images, read-only. A plain config.json,
      # no credential helpers (see "Private registries" below).
      # - /srv/depths/docker/config.json:/run/depths/docker/config.json:ro
    environment:
      # DOCKER_CONFIG: /run/depths/docker
```

- The socket proxy needs nothing new: pulls, tags, inspects and removals are `/images/*`
  (`IMAGES=1`, `POST=1`), already enabled. `DISTRIBUTION` stays off.

## Private registries (user-facing note, also in the guide)

- Depths can only check, and spec 050 can only pull, private images when a plain `config.json`
  is mounted into the Depths container and `DOCKER_CONFIG` names its directory. The Docker daemon
  holds no registry credentials, and the container can't see the host's `~/.docker`.
- Credential helpers (`credsStore`, `credHelpers`) are not supported: the helper programs are not
  in the image. Services on those registries show "can't check", and 050's Pull fails for them
  the same way. Identity tokens (`identitytoken`) are not supported for checks either.
- To make a plain file, log in into a separate directory:
  `DOCKER_CONFIG=/srv/depths/docker docker login --username <user> --password-stdin <registry>`,
  then mount `/srv/depths/docker/config.json` read-only. If the login still hands the password to
  a helper installed on the host (the file then names `credsStore`), write the file by hand
  instead: `{"auths":{"<registry>":{"auth":"<base64 of user:token>"}}}`.
- The file holds passwords in base64. Mount it read-only, readable only by the Depths user.


## Startup log

One line at start (JSON log, never values from `config.json`):

`updates: feature=on auto=on checks=off zone=Europe/Berlin docker_config=/run/depths/docker (2 registries)`
