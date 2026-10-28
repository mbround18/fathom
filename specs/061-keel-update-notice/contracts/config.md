# Contract: update-check configuration (spec 061)

These settings sit alongside the others in `apps/keel/server/src/config.rs`. Each operator setting
also has a flag. As elsewhere, a flag beats the variable, and the variable beats the default. The
resolved value and its source show in the startup log. The password never shows.

## Operator settings

| Variable | Flag | Default | Meaning |
|---|---|---|---|
| `KEEL_UPDATE_CHECK` | `--update-check` | `false` | `true` or `false` (`parse_bool`). False means Keel makes no registry request at all (FR-004, SC-001) |
| `KEEL_UPDATE_INTERVAL` | `--update-interval` | `6h` | `parse_duration` (`90m`, `6h`, `1d`). Minimum `1h` |
| `KEEL_UPDATE_IMAGE` | `--update-image` | `docker.io/mbround18/keel:latest` | The watched reference: `[registry/]repository[:tag]`. A bare name means `docker.io`, and no tag means `latest` |
| `KEEL_UPDATE_REGISTRY_USERNAME` | `--update-registry-username` | unset | For private repositories, or a higher rate limit |
| `KEEL_UPDATE_REGISTRY_PASSWORD` or `KEEL_UPDATE_REGISTRY_PASSWORD_FILE` | none | unset | A secret: read from the env or a file only, never from a flag, never logged |
| `KEEL_IMAGE_DIGEST` | `--image-digest` | unset | The running image's digest, as deployed (R3). Set by the `keel_server` role |

## Image-provided settings

These are set by the Dockerfile's runtime stage. They are not operator settings, and have no
flags.

| Variable | Meaning |
|---|---|
| `KEEL_BUILD_VERSION` | Release version, or empty for untagged builds |
| `KEEL_BUILD_REVISION` | Source revision, for example `cf7fdd6` or `cf7fdd6-dirty` |
| `KEEL_BUILD_DATE` | RFC 3339 UTC build time |
| `KEEL_BUILD_SOURCE` | Source repository URL |

## Validation

Keel refuses to start, naming the setting (FR-009), when:

- `KEEL_UPDATE_CHECK` isn't a boolean;
- `KEEL_UPDATE_INTERVAL` doesn't parse, or is under `1h`;
- `KEEL_UPDATE_IMAGE`:
  - doesn't parse as a reference;
  - carries a digest (`@sha256:…`): a digest never changes, so watching one is a mistake;
  - names an `http://` registry;
  - has an upper-case repository;
  - has a tag longer than 128 characters;
- only one of the username and the password is set;
- `KEEL_UPDATE_REGISTRY_PASSWORD` and `_FILE` are both set, or the file can't be read;
- `KEEL_IMAGE_DIGEST` isn't `sha256:` followed by 64 lowercase hex characters;
- `KEEL_BUILD_DATE` isn't RFC 3339;
- `KEEL_BUILD_REVISION` isn't 7–40 hex characters, with an optional `-dirty`;
- `KEEL_BUILD_SOURCE` isn't an `https` URL.

The interval, image and registry settings are validated even when checks are off. A typo
shouldn't wait for the day someone turns checks on.

## Schedule

- The first check runs 60–120 s after start, at a random point.
- Later checks run every `KEEL_UPDATE_INTERVAL`, with ±5% jitter.
- After a failure, there is one retry after 5 minutes, then the normal schedule.
- A manual check doesn't move the schedule.

## Compose example

```env
KEEL_UPDATE_CHECK=true
# KEEL_UPDATE_INTERVAL=6h
# KEEL_UPDATE_IMAGE=docker.io/mbround18/keel:latest
# KEEL_IMAGE_DIGEST is written by the keel_server role; set it by hand only without the role.
```

`apps/keel/compose.yaml` gains these keys in its environment list, passed through from the host
`.env` like the others.
