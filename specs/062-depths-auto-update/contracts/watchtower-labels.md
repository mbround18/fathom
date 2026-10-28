# Contract: Watchtower label compatibility (062, FR-019–FR-022, SC-006)

Implemented as a pure function in `depths-core` (`updates::labels::effective`), table-tested with
the vectors below. Every vector is one unit test; SC-006's "100% of the compatibility test cases"
means all of them pass.

## Inputs

- `labels`: the running container's labels (they include labels inherited from the image, as
  Watchtower sees them); for a service with no container, the service's labels from
  `compose config --format json`.
- `stack_policy`: `off | notify | auto` (default `notify`).
- Host settings: `label_only: bool`, `scope: Option<String>`.

## Keys

Prefix `P = com.centurylinklabs.watchtower`.

| Key | Values | Watchtower behaviour | Depths behaviour |
|---|---|---|---|
| `P.enable` | bool | `false`: never updated. With `--label-enable`: only `true` is updated. | Same. `false` → excluded `label_disabled`. With `label_only`: not `true` → excluded `label_only_mode`. |
| `P.monitor-only` | bool | Checked and notified, never updated. | `true` caps the policy at `notify`. |
| `P.scope` | string | With `--scope X`: only containers with scope `X`. `--scope none`: only containers without a scope label or with `none`. No `--scope`: all. | Same, with Depths' `scope` setting in place of `--scope`. Excluded `scope_mismatch`. |
| `P.no-pull` | bool | Never pulls; only restarts on a locally changed image. | `true` → excluded `no_pull` (Depths only updates by pulling). |
| `P.lifecycle.pre-check`, `.post-check`, `.pre-update`, `.post-update` | command | Runs the command in the container. | Shown `not_run`: hooks are commands inside the container, allowed only through the guarded shell (051). |
| `P.lifecycle.pre-update-timeout`, `.post-update-timeout` | minutes | Hook timeout. | Shown `not_run` with its hook. |
| `P.depends-on` | names | Restarts listed containers in order. | Shown `not_run`: order comes from the compose file. |
| `P.stop-signal` | signal | Signal used to stop. | Shown `not_used`: compose's `stop_signal` applies. |
| `P` (bare) = `true` | — | Marks the Watchtower container itself. | Watchtower detection (below). |
| any other `P.*` | — | — | Shown `unrecognised`. |

Booleans: Go `strconv.ParseBool`: `1 t T TRUE true True` → true, `0 f F FALSE false False` →
false. Anything else (`yes`, `on`, ` true`, empty) is treated as absent and shown `unrecognised`.
Scope comparison is exact and case-sensitive, as in Watchtower.

## Combination

1. Exclusions first, in this order (first match is the reason shown): `enable=false`,
   `label_only` without `enable=true`, scope mismatch, `no-pull=true`.
2. Otherwise the policy is the stack policy, capped at `notify` by `monitor-only=true`.
3. A stack policy of `off` stays `off` whatever the labels say (labels never widen).
4. `source` names what decided: `label:<key>` when a label made the result more restrictive than
   the stack policy, otherwise `stack_policy` or `default`.

## Test vectors

`—` = label absent. Results: `X(reason)` excluded, otherwise the effective policy and source.

| # | enable | monitor-only | scope | no-pull | label_only | Depths scope | stack policy | Result |
|---|---|---|---|---|---|---|---|---|
| 1 | — | — | — | — | false | — | notify (default) | notify, default |
| 2 | — | — | — | — | false | — | auto | auto, stack_policy |
| 3 | `false` | — | — | — | false | — | auto | X(label_disabled) |
| 4 | `False` | — | — | — | false | — | notify | X(label_disabled) |
| 5 | `0` | — | — | — | false | — | auto | X(label_disabled) |
| 6 | `true` | — | — | — | false | — | auto | auto, stack_policy |
| 7 | — | — | — | — | true | — | auto | X(label_only_mode) |
| 8 | `true` | — | — | — | true | — | auto | auto, stack_policy |
| 9 | `false` | — | — | — | true | — | auto | X(label_disabled) |
| 10 | `yes` | — | — | — | true | — | auto | X(label_only_mode); `enable` shown unrecognised |
| 11 | `yes` | — | — | — | false | — | auto | auto, stack_policy; `enable` shown unrecognised |
| 12 | — | `true` | — | — | false | — | auto | notify, label:monitor-only |
| 13 | — | `true` | — | — | false | — | notify | notify, stack_policy |
| 14 | — | `t` | — | — | false | — | auto | notify, label:monitor-only |
| 15 | — | `false` | — | — | false | — | auto | auto, stack_policy |
| 16 | — | `true` | — | — | false | — | off | off, stack_policy |
| 17 | — | — | `media` | — | false | — | auto | auto, stack_policy; scope `media` shown |
| 18 | — | — | `media` | — | false | `media` | auto | auto, stack_policy |
| 19 | — | — | `media` | — | false | `infra` | auto | X(scope_mismatch) |
| 20 | — | — | — | — | false | `media` | auto | X(scope_mismatch) |
| 21 | — | — | — | — | false | `none` | auto | auto, stack_policy |
| 22 | — | — | `none` | — | false | `none` | auto | auto, stack_policy |
| 23 | — | — | `media` | — | false | `none` | auto | X(scope_mismatch) |
| 24 | — | — | `Media` | — | false | `media` | auto | X(scope_mismatch) |
| 25 | — | — | — | `true` | false | — | auto | X(no_pull) |
| 26 | `true` | `true` | `media` | — | true | `media` | auto | notify, label:monitor-only |
| 27 | `false` | `true` | `media` | `true` | true | `infra` | auto | X(label_disabled) |
| 28 | — | — | `media` | `true` | false | `infra` | auto | X(scope_mismatch) |

Hook and order labels (independent of the policy result):

| # | Label | Shown as |
|---|---|---|
| 29 | `P.lifecycle.pre-update=/backup.sh` | not_run: "lifecycle hooks run commands in the container; Depths only runs commands through the guarded shell" |
| 30 | `P.lifecycle.post-check=…`, `P.lifecycle.post-check-timeout=5` | both not_run |
| 31 | `P.depends-on=db,cache` | not_run: "order comes from the compose file" |
| 32 | `P.stop-signal=SIGHUP` | not_used: "compose's stop_signal applies" |
| 33 | `P.foo=bar` | unrecognised |

## Detecting a running Watchtower

A running container is Watchtower when either:
- it has label `com.centurylinklabs.watchtower=true` (any boolean true form); or
- its image repository, after normalisation, is one of: `docker.io/containrrr/watchtower`,
  `ghcr.io/containrrr/watchtower`, `docker.io/nickfedor/watchtower`,
  `ghcr.io/nicholas-fedor/watchtower`, `docker.io/beatkind/watchtower` (any tag or digest).
  The fork list is confirmed against the forks' published images at implementation.

| # | Container | Watchtower? |
|---|---|---|
| 34 | image `containrrr/watchtower:latest`, no labels | yes |
| 35 | image `ghcr.io/containrrr/watchtower@sha256:…` | yes |
| 36 | image `myorg/watchtower-dashboard`, no labels | no |
| 37 | image `custom/updater`, label `com.centurylinklabs.watchtower=true` | yes |
| 38 | image `containrrr/watchtower`, state exited | no (only running containers count) |

Detection reads the snapshot the runtime already keeps (`containers_by_project`, including
standalone containers); it adds no polling. Depths never stops, restarts or relabels the container.
