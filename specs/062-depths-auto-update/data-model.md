# Data model: Depths image updates

Entities from the spec's Key Entities, as stored and as sent. Rust shapes are the protocol types
in `crates/depths-protocol` unless marked "server only". JSON is what lands on disk.

## Image reference (core, not stored)

```rust
pub struct ImageRef {
    pub registry: String,      // "docker.io", "ghcr.io", "registry.example:5000"
    pub repository: String,    // "library/nginx"
    pub tag: Option<String>,   // "1.27" ; None only when pinned by digest
    pub digest: Option<String>,// "sha256:…" when the compose reference pins one
}
```

- Parsed from the compose `image:` with Docker's normalisation: no registry → `docker.io`;
  single-component Docker Hub repo → `library/<name>`; no tag and no digest → `latest`.
- depths-core's own, pure type: it decides exclusions and keep-tag names without I/O. The server
  converts it to `oci_registry::ImageRef` for requests (contracts/registry.md §3); a test runs the
  table below through both parsers and requires the same repository and reference, and the same
  registry once `docker.io` is read as the API host `registry-1.docker.io`.
- `digest.is_some()` → excluded `pinned_by_digest` (FR-004).
- Table tests cover: `nginx`, `nginx:1.27`, `library/nginx`, `docker.io/nginx`,
  `index.docker.io/library/nginx`, `ghcr.io/a/b:c`, `localhost:5000/x`, `host:5000/a/b@sha256:…`,
  `x:1@sha256:…` (both: digest wins, excluded), upper case (invalid → excluded `bad_reference`).

## Host update settings (`updates.json`)

```json
{
  "version": 1,
  "settings": {
    "checks_enabled": false,
    "interval_secs": 21600,
    "window": { "days": ["mon","tue","wed","thu","fri","sat","sun"], "start": "03:00", "end": "05:00" },
    "scope": null,
    "label_only": false,
    "cleanup": false,
    "health_timeout_secs": 120,
    "changed_by": { "provider": "github", "subject": "123", "username": "ana" },
    "changed_at": "2026-10-28T09:00:00Z"
  },
  "policies": { "…": "see Update policy" },
  "rejected": [ "…see Rejected digest" ],
  "watchtower_acks": [ "…see Watchtower acknowledgement" ]
}
```

Validation:
- `interval_secs` ≥ 3600 (FR-002); default 21600.
- `health_timeout_secs` 30..=1800; default 120 (FR-014).
- `window`: `start`/`end` are `HH:MM` 00:00–23:59; `days` non-empty, no duplicates; `end == start`
  is refused (an empty or 24 h window is ambiguous; a full day is `00:00`–`23:59`).
- `scope`: `null` or 1..=64 chars `[A-Za-z0-9._-]`; the literal `none` has Watchtower's meaning.
- Unknown fields refused. An unreadable file is never rewritten: Depths runs with checks off and
  Auto paused, and Updates shows the path and the parse error (research R5).

## Update policy (`updates.json` → `policies`)

```json
"policies": {
  "media": {
    "policy": "auto",
    "window": null,
    "set_by": { "provider": "github", "subject": "123", "username": "ana" },
    "set_at": "2026-10-28T09:05:00Z"
  }
}
```

```rust
pub enum UpdatePolicy { Off, Notify, Auto }   // serde snake_case
pub struct StackUpdatePolicy {
    pub policy: UpdatePolicy,
    pub window: Option<Window>,               // overrides the host window
    pub set_by: Actor,
    pub set_at: String,                       // RFC 3339 UTC
}
```

- A stack with no entry is `Notify` (FR-012). Only stacks an admin changed are stored.
- Kept by stack name; a stack whose directory is gone keeps its entry, shown "missing".
- `set_by` is the actor named as `on_behalf_of` in scheduled audit entries (FR-016).
- An `auto` policy whose `set_by` is no longer an administrator in `users.json` is paused:
  computed at each scheduler pass, not stored, sent as `UpdatesView.paused_policies`, shown
  "author no longer an administrator". Another admin saving the policy makes them `set_by` and
  lifts the pause (FR-025).

## Effective policy (computed, not stored)

```rust
pub struct EffectivePolicy {
    pub policy: UpdatePolicy,           // after labels: never less restrictive than the stack's
    pub source: PolicySource,           // StackPolicy | Default | Label(LabelRule)
    pub excluded: Option<Exclusion>,
    pub labels: Vec<WatchtowerLabel>,   // every watchtower label seen, with what Depths did
}
pub enum Exclusion {
    PinnedByDigest, BuildContext, LocalImage, BadReference,
    LabelDisabled, LabelOnlyMode, ScopeMismatch { scope: Option<String> },
    NoPull, DepthsItself,
}
pub struct WatchtowerLabel { pub key: String, pub value: String, pub effect: LabelEffect }
pub enum LabelEffect { Applied, NotRun { reason: String }, NotUsed { reason: String }, Unrecognised }
```

Restriction rank, most restrictive first: excluded, `Off`, `Notify`, `Auto`. The most restrictive of stack policy and labels wins (FR-019). `DepthsItself` (FR-024) is set for
the container Depths runs in (`HOSTNAME` match, as `files.rs` already does).

## Service update state

Stored in `update-state.json` (server cache), sent in `UpdatesView` and `StackDetail`.

```rust
pub struct ServiceUpdate {
    pub stack: String,
    pub service: String,
    pub reference: String,               // as written in the compose file
    pub running: Option<RunningImage>,   // None when no container
    pub available: Option<AvailableImage>,
    pub state: UpdateState,
    pub reason: Option<String>,          // human text for held / excluded / failed / check_failed ("can't check: …")
    pub effective: EffectivePolicy,
    pub last_checked: Option<String>,
    pub last_outcome: Option<StackOutcome>,
    pub replicas: u32,
}
pub struct RunningImage { pub image_id: String, pub repo_digest: Option<String>, pub version: Option<String>, pub revision: Option<String> }
pub struct AvailableImage {
    pub digest: String,                  // what the tag points to (index or manifest)
    pub platform_digest: String,         // manifest for the host platform
    pub config_digest: String,
    pub version: Option<String>,         // org.opencontainers.image.version
    pub revision: Option<String>,        // org.opencontainers.image.revision
    pub created: Option<String>,
    pub user: Option<String>,            // config.User, for the root check (R4)
}
pub enum UpdateState {
    NotChecked, UpToDate, Available, MovedBack, Rejected,
    Held, Excluded, Updating, Failed, RolledBack, RollbackFailed,
    Interrupted, CheckFailed,
}
```

State transitions (per service):

```
NotChecked ─check─▶ UpToDate | Available | MovedBack | Rejected | CheckFailed | Excluded
Available ─run─▶ Updating ─▶ UpToDate (updated) | Held | Failed | RolledBack | RollbackFailed
Updating ─Depths restarts─▶ Interrupted ─next check─▶ UpToDate | Available | …
CheckFailed ─check ok─▶ (normal); CheckFailed keeps last known running/available fields (FR-005)
RolledBack ─same digest─▶ Rejected ; ─newer digest─▶ Available (FR-015)
```

`CheckFailed` covers both a passing failure (rate limit, unreachable) and a "can't check" one
(no digest on HEAD, unsupported credentials); the reason says which. Neither ever pulls.

`Failed` = the update did not reach the health wait (pull or `up` failed) and nothing was
recreated, or `up` failed before rollback was needed.

## Digest resolution cache (`update-state.json` → `resolutions`)

```json
"resolutions": {
  "docker.io/library/nginx@sha256:aaa…": {
    "platform": "linux/amd64",
    "platform_digest": "sha256:bbb…",
    "config_digest": "sha256:ccc…",
    "user": "", "version": "1.27.2", "revision": "abc123", "created": "2026-10-20T…Z",
    "resolved_at": "2026-10-28T03:00:00Z"
  }
}
```

Content addressed: never refreshed. Bounded to 2,000 entries, least recently used evicted.

## Registry backoff (`update-state.json` → `backoff`)

`{ "<registry host>": { "until": "<rfc3339>", "failures": 3, "last_error": "429 Too Many Requests" } }`

## Rejected digest (`updates.json` → `rejected`)

```json
{ "stack": "media", "service": "app", "digest": "sha256:…", "at": "…", "reason": "unhealthy after 120 s",
  "run_id": "…" }
```

- Matched on stack + service + tag digest. A newer digest for the tag makes the service
  `Available` again (FR-015). An admin's **Retry** removes the entry (audited `update_retry`).
- Kept in `updates.json` (an administrator decision follows from it), not the cache.

## Watchtower acknowledgement (`updates.json` → `watchtower_acks`)

```json
{ "container_id": "…64 hex…", "image_id": "sha256:…", "name": "watchtower",
  "answer": "stopped" | "monitor_only", "by": { "…": "Actor" }, "at": "…" }
```

Auto stays paused while any running Watchtower container has no matching acknowledgement.

## Update run (`update-state.json` → `journal`, and the audit log)

```rust
pub struct UpdateRun {
    pub run_id: RunId,                      // uuid
    pub trigger: RunTrigger,                // Manual { actor } | Scheduled { on_behalf_of: per stack }
    pub stacks: Vec<StackRun>,              // in order
    pub started_at: String,
    pub finished_at: Option<String>,
}
pub struct StackRun {
    pub stack: String,
    pub services: Vec<ServiceChange>,
    pub operation_id: Option<OperationId>,  // ties to the streamed output
    pub phase: RunPhase,                    // Pending, Pulling, Checking, Recreating, Waiting, RollingBack, Cleaning, Done
    pub outcome: Option<StackOutcome>,
}
pub struct ServiceChange { pub service: String, pub from_image: String, pub to_digest: String, pub to_image: Option<String> }
pub enum StackOutcome { Updated, Held, Skipped { reason: String }, Failed, RolledBack, RollbackFailed, Interrupted, Stale }
```

- Journal holds the open runs and the last 50 finished runs (Updates "recent runs").
- The phase is written (fsync) before each mutating step; an open stack run found at start becomes
  `Interrupted` (FR-018, research R5).

## Auto last run (`update-state.json` → `auto_last_run`)

`{ "<stack>": "2026-10-28" }`: per stack, the window occurrence the Auto queue last ran it in,
keyed by the local date the occurrence opened (depths-core's `OccurrenceKey`).

- Written (atomic, `0600`, fsync) before the stack runs, so a failed, held or interrupted stack
  waits for the next occurrence, and a restart inside the same window does not run it again.
- Read once when the Auto task starts. Missing, from before the field, or unreadable: no record,
  with a warning; start-up never fails over it. An unreadable `auto_last_run` alone is dropped and
  the rest of the file kept; an unreadable file is moved aside as above.
- A digest Auto rolled back is not here: it is in `updates.json → rejected`, which a restart reads.

## Keep tags (on the Docker host)

`depths-keep/<stack>:<service>-previous` and `depths-keep/<stack>:<service>-held`, names sanitised
as in research R7. Depths records each tag it set in `update-state.json → keep` with the image ID,
so cleanup only ever touches what it recorded.

- Visible in `docker images`; documented in the guides (FR-028).
- Cleanup never removes an image a keep tag records as a stack's current rollback target (FR-027).

## Audit additions

```rust
pub enum Provider { Oidc, Github, Discord, Depths }   // Depths: server-created actors only
pub struct AuditEntry {
    // … existing …
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_behalf_of: Option<Actor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub update: Option<AuditUpdate>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub change: Option<AuditChange>,       // settings / policy before & after
}
pub struct AuditUpdate { pub run_id: RunId, pub services: Vec<AuditServiceUpdate>, pub rejected_digest: Option<String>, pub hold: Option<String> }
pub struct AuditServiceUpdate { pub service: String, pub from_image: String, pub from_digest: Option<String>,
                                pub to_image: Option<String>, pub to_digest: String, pub version: Option<String> }
pub struct AuditChange { pub before: serde_json::Value, pub after: serde_json::Value }
```

New `AuditAction` variants: `image_update`, `image_rollback`, `image_hold`, `image_hold_apply`,
`image_remove`, `update_interrupted`, `update_settings`, `update_policy`, `update_retry`,
`watchtower_ack`. Check runs are not audited (no mutation); their results are in the state file.
