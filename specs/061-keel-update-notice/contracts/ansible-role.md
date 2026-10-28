# Contract: `keel_server` role changes (spec 061, FR-003)

The role lives in the owner's Ansible repository (`roles/keel_server`, spec 058), not in this
tree. This contract is what the role must do. The owner makes the change in that repository.
Nothing in this spec edits it.

## Today (spec 058)

- `keel_server_image: mbround18/keel:latest`.
- Outside Keel, `docker_compose_v2` runs with `pull: missing`. Under Keel, the role pulls
  (`policy: missing`), plans in check mode, then defers `docker compose up -d` by 30 s with
  `systemd-run`.
- `compose.yaml.j2` uses `image: {{ keel_server_image }}`.
- With `pull: missing` and a moving tag, a newer `latest` is never picked up once any `latest` is
  present.

## Variables

| Variable | Default | Meaning |
|---|---|---|
| `keel_server_image` | `mbround18/keel:latest` | Unchanged. The repository and tag to follow |
| `keel_server_image_digest` | `""` | When set, deploy exactly `<repository>@<digest>`. Passed by Keel as a job param (R4), or by hand for rollback |
| `keel_server_update_check` | `false` | Written as `KEEL_UPDATE_CHECK` |

The role derives `keel_server_image_repository` by stripping the tag from `keel_server_image`, and
refuses an image that already carries `@sha256:`.

## Behaviour

1. **Validate**: if `keel_server_image_digest` is set, it must match `^sha256:[0-9a-f]{64}$`.
   Otherwise the role fails before touching the host.
2. **Resolve the target**:
   - **Digest given**: `target = {{ keel_server_image_repository }}@{{ keel_server_image_digest }}`.
     Pull it with `policy: missing`. A digest never moves, so `missing` is correct.
   - **No digest**: pull `{{ keel_server_image }}` with `policy: always`, which fixes the
     stale-`latest` problem. Then read `RepoDigests` for that repository from `docker image
     inspect`, and set
     `target = {{ keel_server_image_repository }}@<that digest>`.
   - In both cases, check mode reports what would change without pulling.
3. **Record the previous digest**: read the current `KEEL_IMAGE_DIGEST` from
   `{{ keel_server_dir }}/keel.env`, if present. If it differs from the target, write it to
   `{{ keel_server_dir }}/digest.previous` (mode 0644, root-owned) before changing anything.
4. **Render**:
   - `compose.yaml.j2`: the `image:` line becomes `{{ keel_server_target_image }}`, always pinned
     to a digest.
   - `keel.env.j2`: gains `KEEL_IMAGE_DIGEST={{ digest }}` and
     `KEEL_UPDATE_CHECK={{ keel_server_update_check | lower }}`.
   - `{{ keel_server_dir }}/digest.current` holds the deployed digest.
5. **Apply**: as today. Outside Keel, `docker compose up -d`. Under Keel, the deferred
   `systemd-run` restart, so the run reports its result before Keel restarts. Spec 060's runner
   replay covers a result that is late anyway.
6. **Idempotence**: with the same target, nothing changes, and the run reports `ok`.

## Rollback

```sh
make keel-server KEEL_DIGEST=$(cat /opt/keel/digest.previous)
```

The repository's Makefile has no extra-vars hook today. The `keel-server` target gains one:
`$(if $(KEEL_DIGEST),-e keel_server_image_digest=$(KEEL_DIGEST))`. This is the break-glass path
from spec 058. Keel writes `keel_updated` on its next start (data-model.md).

## Manifest

The `keel-server` job in `.keel/jobs.yaml` declares the param:

```yaml
params:
  keel_server_image_digest: { kind: oci_digest }
```

Add it **only after** the deployed Keel and runner support params. An older parser rejects
unknown keys. quickstart.md gives the order.

## Tests in that repository

Its existing test pattern is `scripts/tests` with pytest. Cases:

- digest validation;
- repository derivation (`repo:tag`, `registry:5000/repo:tag`, refusing `repo@sha256:…`);
- `digest.previous` written only on change;
- the rendered compose `image:` always contains `@sha256:`;
- check mode makes no pull.
