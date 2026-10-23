# Data Model: Depths (050)

Types live in `crates/depths-core` (domain) and are mirrored on the wire in
`crates/depths-protocol` (ts-rs). Container-level types (`Container`, `Image`, `OperationPreview`
pieces, stats samples) are the 048 wire types from `crates/container-runtime`, reused unchanged.

## StacksRoot

| Field | Type | Rule |
|---|---|---|
| path | absolute path | From `--stacks-dir` / `DEPTHS_STACKS_DIR`, default `/opt/stacks`. Canonicalised at start. Must exist and be a directory, or Depths refuses to start with a clear message. |
| global_env | bool | `path/global.env` exists (R1). Re-checked on every plan. |

## Stack

One immediate sub-directory of the stacks root.

| Field | Type | Rule |
|---|---|---|
| name | string | Directory name. `valid_name` = matches `^[a-z0-9][a-z0-9_-]*$`. |
| compose_file | enum `compose.yaml \| compose.yml \| docker-compose.yml \| docker-compose.yaml` or none | The first present in Compose's order (R1). None → not a stack, not listed. |
| shadowed_files | list | Other compose file names present; non-empty → warning (FR-004). |
| has_env | bool | `.env` present. |
| content_hash | sha256 hex | Over compose file bytes + `.env` bytes (+ names). Used for conflict detection and preview binding. |
| status | StackStatus | Derived; see below. |
| unhealthy | bool | Any container's health is `unhealthy`. |
| services | list of ServiceSummary | From the parsed compose file (names, image) joined with containers. |
| running_services / total_services | counts | Long-running services up / services defined (FR-006). |
| modified_at | time | Latest mtime of the compose file and `.env` (FR-006). |
| managed | bool | Always true for a Stack; foreign projects are a separate type. |
| read_error | string or none | The I/O error when the dir or compose file cannot be read; the stack is listed, never edited. |
| read_only_reason | string or none | Set when Depths cannot write the files, or the stack dir is a symlink. |
| too_large | bool | A compose or `.env` file exceeds `DEPTHS_MAX_FILE_BYTES` (default 1 MiB); shown, not loaded into the editor. |

**Invariants**

- A stack's path is `root/name` and every file Depths touches canonicalises to a path whose parent
  is exactly `root/name` (no `..`, no absolute names, no symlink whose target leaves the stack dir;
  FR-005). A symlinked stack *directory* is listed read-only with a reason.
- `invalid` stacks (bad name, unparseable compose) are listed and never acted on.

### StackStatus (derived, R2)

Computed from the containers with label `com.docker.compose.project = name`:

```
no containers                                  → not_deployed
all long-running services running              → running
some running, some not                         → partial
none running                                   → stopped
                                                 (not_deployed is how the UI shows a stopped stack with no containers; FR-006's four statuses plus that label)
compose file unparseable or name invalid       → invalid
```

A container that `exited` with code 0 whose restart policy is `no` counts as **completed** and is
excluded from "long-running" (it never makes a stack partial). A container in `restarting` counts
as not running.

## ForeignProject

A compose project seen in containers or `docker compose ls --all` with no matching stack
directory (or whose `working_dir` label is outside the root). Fields: `name`, `working_dir`,
`containers`, `elsewhere_of`. Read-only for every role (FR-008). When `working_dir` is a stack's directory but the project name differs (started by hand with `-p`), it carries `elsewhere_of: <stack>` and the stack shows "running elsewhere". Plain containers with no project label are listed
as `standalone` under the same read-only rule.

## StackDraft (editing)

| Field | Type | Rule |
|---|---|---|
| stack | name | Existing stack, or a new name for create. |
| base_hash | sha256 or none | The `content_hash` the editor loaded; none for create. |
| compose | string | Exact text; written byte for byte. |
| env | string or none | Exact text; none = leave `.env` as is; empty string with no existing file = do not create. |

**Validation** (on save preview): name valid and not taken (create); YAML parses (client and
server); `docker compose -f - config --quiet` exits 0 with cwd = stack dir (errors returned with
line numbers when Compose gives them); `base_hash` equals the current hash, else `Conflict`
carrying the current text (FR-019).

## Operation

| Field | Type | Rule |
|---|---|---|
| id | uuid | Server-generated. |
| kind | `up \| stop \| restart \| down \| pull \| update \| save \| create \| delete \| service_up \| service_stop \| service_restart` | |
| stack | name | |
| options | per kind | `down`: `remove_volumes`, `remove_images` (default false, named in the preview when on; FR-015). `save`/`create`: `deploy` (save and deploy: diff and `up` in one preview; FR-020). `delete`: `remove_directory` (default false). service ops: `service`. |
| preview | Preview | Built by the server. |
| state | see transitions | |
| actor | Identity | |

**Preview**

| Field | Type |
|---|---|
| preview_id | uuid, single use |
| steps | list of `{argv: [string], cwd: path, stdin: "draft" \| none}` |
| file_diff | unified diff, for save/create/delete |
| reaches | containers, networks, volumes (only when chosen) the op affects |
| destructive | bool |
| bound_to | `{session_id, stack_hash, expires_at}` (5 min) |

**State transitions**

```
requested ─preview built─▶ previewed ─confirm(preview_id)─▶ running ─▶ succeeded
                │                │                              └────▶ failed (stderr verbatim)
                │                └─expired / hash changed / session ended─▶ void
                └─role denied─▶ denied (audited)
```

At most one operation per stack is `running`; a second request gets `Busy{operation_id, actor}`
(FR-014). Operations on different stacks run concurrently.

## Identity and Session

From `web-auth`. `Identity{provider: oidc|github|discord, issuer, subject, username, email, groups}`.
GitHub groups are `github:<org>` and `github:<org>/<team>` (R5). Discord groups are
`discord:<guild_id>`, `discord:<guild_id>/<role_id>` and `discord:user/<user_id>` (R13). Session: id (cookie
`__Host-depths_session`, HttpOnly, Secure, SameSite=Lax), csrf token, identity, expires_at
(idle 8 h, absolute 24 h, configurable), next_refresh_at (15 min).

## RoleGrant and effective role

```
RoleGrant { group: string, role: viewer|operator|admin, stacks: "*" | [pattern] }
```

`role(identity, stack) = max{ g.role | g.group ∈ identity.groups ∧ g.stacks matches stack }`,
none if no grant matches. Permissions:

| Action | viewer | operator | admin |
|---|---|---|---|
| See stack, files (values of secret-looking keys masked, FR-021), status, logs, stats | ✓ | ✓ | ✓ |
| Reveal masked `.env` values | | ✓ | ✓ |
| Lifecycle (up/stop/restart/down/pull/update, per service) | | ✓ | ✓ |
| Shell (when `DEPTHS_FEATURE_SHELL`) | | ✓ | ✓ |
| Edit/save, create, delete stack; `down` with volumes or images (FR-016) | | | ✓ (`*` grant required for create) |
| Read audit log, see roles mapping | | | ✓ |

A person with no role on any stack signs in and sees the no-access page (FR-031).

## AuditEntry

Appended by `audit-log` as one JSON line.

| Field | Type |
|---|---|
| at | RFC 3339 UTC |
| actor | `{provider, subject, username}` |
| action | Operation kind, or `sign_in`, `sign_out`, `denied`, `shell_open`, `reveal_env` |
| stack | name or none |
| preview_id / operation_id | uuid or none |
| argv | the steps run |
| outcome | `succeeded \| failed{exit_code, stderr_tail} \| denied{reason}` |
| source_ip | string |

Never contains `.env` values or file contents: saves record hashes before/after and a diff summary (files and lines changed, with lines in `.env` reduced to their keys; FR-021, FR-035). Rotation by size into
`audit-log.<n>.jsonl`; entries are never rewritten or removed by Depths (FR-036).

## Tab (web only)

`{id, kind: logs|shell|stats, target: {stack, service?, container}, pinned}`; layout
`{panes: [[tab_id]], sizes: [number]}` persisted in `localStorage` keyed by `depths:layout:<subject>`
(per person, per browser). A reloaded shell tab reconnects to a **new** session (shells are not
resumable); logs resume with `--since` the last seen timestamp; buffers are rings of 10,000 lines.
