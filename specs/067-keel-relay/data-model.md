# Data model: Keel relays (spec 067)

Migration `apps/keel/server/migrations/0015_relays.sql`. Grants follow the existing pattern:
`keel_app` gets `select, insert, update`; no `delete` or `truncate` on keys and relays (revoke
instead of delete, like `ssh_cas`).

## Tables

### `zones`

| Column | Type | Notes |
|---|---|---|
| `id` | `uuid` PK | |
| `workspace_id` | `uuid` not null → `workspaces(id)` on delete cascade | |
| `name` | `text` not null | `^[a-z0-9][a-z0-9-]{0,62}$`. `direct` is reserved (the implicit zone). |
| `description` | `text` not null default `''` | ≤ 500 chars. |
| `created_at` | `timestamptz` not null default `now()` | |

Unique `(workspace_id, name)`. A zone with hosts or non-revoked relays cannot be removed
(the preview lists them).

### `hosts` (altered)

| Column | Type | Notes |
|---|---|---|
| `zone_id` | `uuid` null → `zones(id)` | Null: the implicit `direct` zone. Must be in the host's workspace (checked in the transaction). |
| `last_path` | `jsonb` null | The path of the last connected session: `{"kind":"direct","peer":"203.0.113.4:51234"}` or `{"kind":"relay","relay":"<uuid>","peer":"10.0.0.5:40122"}`. `peer` under a relay is what the relay claimed (advisory). |

### `relay_link_keys`

Keel's own key for dialling relays (research R2). Shaped like `ssh_cas`.

| Column | Type | Notes |
|---|---|---|
| `id` | `uuid` PK | |
| `public_key` | `text` not null | OpenSSH line. |
| `fingerprint` | `text` not null unique | `SHA256:…` |
| `sealed_private_key` | `bytea` not null | Under the master key, aad `relay_link_key:<id>`. |
| `state` | `text` not null | `pending` \| `active` \| `retired`. |
| `created_at`, `activated_at`, `retired_at` | `timestamptz` | |

Unique partial index: one `active` row. Created on first start when none exists.

### `relays`

| Column | Type | Notes |
|---|---|---|
| `id` | `uuid` PK | |
| `workspace_id` | `uuid` not null → `workspaces(id)` | |
| `zone_id` | `uuid` not null → `zones(id)` | Same workspace. |
| `name` | `text` not null | Same rule as zone names. Unique per workspace. |
| `dial_address` | `text` not null | `host:port`; a name or an IP literal (`[v6]:port`). Keel dials it. |
| `host_key` | `text` not null | The relay's Ed25519 public line, pasted. |
| `host_key_fingerprint` | `text` not null unique | Two relays never share a key. |
| `host_id` | `uuid` null → `hosts(id)` | The Keel host the relay runs on, if any (R10 warning). |
| `created_at`, `updated_at` | `timestamptz` | |
| `revoked_at` | `timestamptz` null | Set once, never cleared. A revoked relay is shown, never dialled. |

Live state (`connecting`, `connected`, `unreachable`, last RTT, version, stream count, last
error, `since`) is **in memory** in the relay supervisor, not in the table: it changes every
30 s and is meaningless after a restart. State **changes** are audited.

### `host_queries`

| Column | Type | Notes |
|---|---|---|
| `id` | `uuid` PK | Also the `query_id` on the wire. |
| `workspace_id`, `host_id` | `uuid` not null | |
| `kind` | `text` not null | `facts` \| `applied_revision` \| `unit_state` \| `disk_health` \| `path_free` |
| `params` | `jsonb` not null default `{}` | `{"unit": "k3s.service"}`, `{"path": "/var/lib/longhorn"}` |
| `asked_by` | `uuid` not null → `users(id)` | |
| `asked_at` | `timestamptz` not null | |
| `outcome` | `text` null | `answered` \| `refused` \| `timeout` \| `no_capability` \| `disconnected` |
| `answer` | `jsonb` null | Bounded to 64 KiB; larger answers are refused by the runner. |
| `refusal` | `text` null | Reason code from the runner. |
| `answered_at` | `timestamptz` null | |

The newest 200 rows per host are kept; older ones are deleted in the same transaction as each
insert (Keel has no housekeeping job to hang this on). Answers name paths and units, nothing
secret. `keel_app` gets `delete` on this table only.

## In-memory state

- **`RelaySupervisor`** (server): one task per non-revoked relay. Holds the russh client handle,
  `RelayLive { state, rtt_ms, version, streams, last_error, since }`, and the set of runner
  connections it is carrying (so revocation can close them).
- **`RunnerPath`** (server): attached to each runner connection, passed to `queue::attach`, kept
  in the run hub's session entry, written to `hosts.last_path` on `Hello`.
- **Relay** (`keel-relay`): link state, the open runner streams (count only for `Pong`), and the
  release cache on disk. Nothing about runners is persisted.

## State transitions

Relay (live):

```text
             register / un-revoke is not possible
 (new) ──► connecting ──handshake+Pong──► connected
              ▲   │                          │
              │   └──fail──► unreachable ◄───┘ missed Pong / link drop
              └──backoff────────┘
 any ──revoke──► revoked (terminal; link and streams closed)
```

Host (display, derived; `hosts.state` itself is unchanged):

| Condition | Shown |
|---|---|
| connected | `connected · direct` or `connected · via <relay>` |
| not connected, zone has a `connected` relay, or zone is `direct` | `disconnected` |
| not connected, every non-revoked relay of its zone `unreachable` | `disconnected (relay unreachable)` |
| not connected, zone has no non-revoked relay and is not `direct` | `no path` |
| last path was a relay that is now revoked | `disconnected (relay revoked)` |

## Validation rules

- `dial_address` parses as `host:port`, port 1–65535. No path, no scheme.
- `host_key` parses as exactly one `ssh-ed25519` public line; any other algorithm is refused
  (keel-exec `KEY_ALGORITHMS`).
- A relay's zone and host (if set) are in the relay's workspace.
- Query `unit` matches `^[A-Za-z0-9@._:-]{1,256}$`; `path` is absolute, normalised, no `..`, ≤ 4096
  bytes. The runner re-checks both against `queries.toml` (Keel's check is for early errors only).

## Audit events (`audit::Entry` actions)

`zone.create`, `zone.update`, `zone.remove`, `host.zone_set`, `relay.register`, `relay.update`,
`relay.revoke`, `relay.state` (connected / unreachable, with reason), `relay.refused`
(handshake with a wrong host key, a refused channel or control frame: what and why),
`relay_link_key.create`, `relay_link_key.activate`, `relay_link_key.retire`, `host.query`
(kind, params, outcome; never the answer body), and on runner sessions the existing connect /
disconnect entries gain `path`.
