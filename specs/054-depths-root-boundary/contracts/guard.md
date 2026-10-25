# Contract: runtime guard (054)

## Topology (compose.yaml)

```
depths ──(depths-guard, internal)── guard ──(docker-api, internal)── socket-proxy ── /var/run/docker.sock:ro
```

- `depths`:
  - sets `DOCKER_HOST=tcp://guard:2375`;
  - joins only the `depths-guard` network, plus its public-facing network;
  - is no longer on `docker-api`.
- `guard`:
  - runs `image: ${DEPTHS_GUARD_IMAGE:-depths-guard:dev}` as `user: 65532:65532`;
  - is hardened: `read_only`, `cap_drop: [ALL]`, `no-new-privileges`, `pids_limit: 64`,
    `mem_limit: 128m`, `restart: ${RESTART_POLICY:-no}`;
  - has the label `org.fathom.depths.protected=true`;
  - has these mounts:
    - `./guard.toml:/etc/depths-guard/guard.toml:ro`;
    - `${STACKS_DIR:-/opt/stacks}:${STACKS_DIR:-/opt/stacks}:ro`, plus every `roots.allow` entry
      read-only at the same path;
    - `./guard-log:/var/log/depths-guard`;
  - uses `depths-guard health` as its healthcheck. The image is scratch, with no shell.
- `socket-proxy`:
  - is unchanged apart from its label `org.fathom.depths.protected=true`;
  - joins only `docker-api`.

## Proxying

- HTTP/1.1, keep-alive.
- Upgraded connections (attach, exec start, `Upgrade: tcp`) are proxied in both directions after
  the guard has judged the request.
- Streaming responses (logs, events, stats) are passed through without buffering.
- A request body is buffered and judged, then forwarded byte for byte. The guard never rewrites a
  request.
- Bodies over 1 MiB are refused, except image pulls, which have no body.

## Refusal response

```
HTTP/1.1 403 Forbidden
Content-Type: application/json

{"message":"depths-guard refused <rule>: <setting> [<project>/<service>]"}
```

- `<setting>` is the field and value that were judged, for example `HostConfig.Privileged=true` or
  `Binds[0]=/etc:/x`. Host paths appear in it. File contents never do.
- `[<project>/<service>]` is left out when the labels are absent.

- If the upstream cannot be reached, the guard answers `502` with
  `{"message":"depths-guard: runtime unreachable"}`.
- If the guard itself cannot be reached, the request fails at the client's connection.

## Guard endpoints (same listener, read-only)

- `GET /_depths_guard/v1/health` → `200 {"ok":true,"rules":<n>,"exceptions":<n>}`.
- `GET /_depths_guard/v1/exceptions[?project=<p>]` →
  `200 [{project, service, image?, grants:[{kind:"device"|"bind"|"cap"|"namespace", value, read_only?}], note?, granted?}]`.

There are no write endpoints. Any other `/_depths_guard/` path answers `404`.

## CLI (`depths-guard`)

- `depths-guard serve` (the default) loads the config, resolves the protected containers, and
  listens. It exits non-zero on a bad config.
- `depths-guard health` exits 0 when `/_depths_guard/v1/health` on localhost answers `ok`.
- `depths-guard check [--config <path>]` judges every existing container's stored config through
  the upstream, as if it were being started.
  - For each container it prints the project, service and rules hit, plus a suggested
    `[[exception]]` TOML block.
  - It exits 1 when any container would be refused. The operator runs it before upgrading
    (FR-023):

    ```
    docker compose run --rm guard check
    ```
- `SIGHUP` reloads `guard.toml`. A bad file keeps the old config and logs the error.
