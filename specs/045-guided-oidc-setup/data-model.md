# Phase 1 data model: guided OIDC setup

Three layers change: the protocol (defined once in Rust, exported to TS), the persisted state (two
additive records), and the UI's draft, which is the only one that is not persisted at all.

## Protocol (`src/protocol/v1/oidc.rs`)

| Type | Fields |
| --- | --- |
| `Discovery` *(existing, extended)* | adds `scopes_supported: Vec<String>`, `claims_supported: Vec<String>`; gains `Serialize` + `TS` beside its existing `Debug, Clone, PartialEq, Eq` |
| `IssuerDraft` *(new)* | `config: OidcIssuerConfig`, `method: OidcSignInMethod` — a whole unsaved issuer, carried inline so sign-in needs no saved record |
| `DraftSignInResult` *(new)* | `username: String`, `groups: Vec<String>`, `claims: Vec<ClaimPair>`, `expires_at: i64` (`#[ts(type = "number")]`) — what the test sign-in proved, with no token material |
| `ApiServerAuthRead` *(new)* | `style: ApiServerAuthStyle`, `issuer: Option<String>`, `client_id: Option<String>`, `username_claim: Option<String>`, `groups_claim: Option<String>`, `username_prefix: Option<String>`, `groups_prefix: Option<String>`, `context: String`, `checked: bool`, `reason: Option<String>` |
| `ApiServerAuthStyle` *(new)* | `Legacy` (`--oidc-*`) \| `Structured` (`--authentication-config`) \| `None` \| `Unknown` — mutually exclusive by construction, so the path cannot write both |
| `ApiServerAuthComparison` *(new)* | `read: ApiServerAuthRead`, `differences: Vec<AuthDifference>`, `writable: bool`, `not_writable_reason: Option<String>` |
| `AuthDifference` *(new)* | `field: String`, `cluster: Option<String>`, `draft: Option<String>` — one row per mismatch; `None` on either side means absent, not empty |
| `IssuerOrigin` *(new)* | `Saved` \| `SeededFromEnvironment` — carried on each entry in the issuer list; the UI renders read-only and names the Secret for the seeded case |

`checked: bool` + `reason: Option<String>` is the three-state "stated unknown" shape of
`detect_kubeadm` (`rbac_access.rs:506`): a successful read, a definite negative, and an
*undetermined* read that carries the API server's verbatim words. `Unknown` is never an assumed pass.

**Invariants**
- `ApiServerAuthStyle::Unknown` implies `checked == false` and `reason.is_some()`.
- `writable == false` whenever provenance (below) is absent, and always in hosted.
- `IssuerOrigin::SeededFromEnvironment` implies the entry is not editable or deletable (FR-032).

## Persisted state

### Kind provenance (new, FR-036)

| Field | Meaning |
| --- | --- |
| `cluster: String` | the kind cluster name, as `kind get clusters` reports it |
| `provisioned_at: i64` | when Fathom created it |
| `profile: Option<String>` | the profile used, which `summaries_from_names` hardcodes `None` today (`kind.rs:225`) |

**Lifecycle**: written when a provision succeeds, removed when Fathom deletes the cluster, and
otherwise never inferred. **Invariant**: a cluster absent from this record is *not owned*, whatever
its context name looks like — that is the whole point of the record.

### Issuer config (existing, additive only)

`OidcIssuerConfig` is unchanged on disk. `IssuerOrigin` is derived, not stored: `Saved` for anything
`load_oidc_config()` returns, `SeededFromEnvironment` for the hosted projection, which is never
written to disk at all.

## Hosted seed (`server/src/oidc_seed.rs`, new)

A pure projection of the already-validated `HostedConfig`, with no new source of truth:

| `OidcIssuerConfig` field | from |
| --- | --- |
| `id` | a fixed slug for the seeded issuer |
| `issuer_url` | `oidc.issuer_raw` (verbatim — `Url` would append a `/`) |
| `client_id`, `scopes` | `oidc.client_id`, `oidc.scopes` |
| `username_claim`, `groups_claim` | `claims.username`, `claims.groups` |
| `username_prefix`, `groups_prefix` | `claims.username_prefix`, `claims.groups_prefix` (empty → `None`) |
| `ca_bundle_path` | `oidc.ca_file` |
| `admin_api_base` | always `None` |

`client_secret` is **not** projected: it is never rendered and never exported (FR-033).

## UI draft (`ui/src/features/identity/guided-state.ts`, new)

```ts
type GuidedStep =
  | 'issuer-url' | 'discovery' | 'claims' | 'test-sign-in'
  | 'cluster-compare' | 'fathom-roles' | 'review-save' | 'bindings' | 'apiserver-write'

type GuidedDraft = {
  step: GuidedStep
  config: OidcIssuerConfig
  discovery?: Discovery
  signIn?: DraftSignInResult
  comparison?: ApiServerAuthComparison
  roleMap: FathomRoleMap
  origin: IssuerOrigin
}
```

**Invariants**
- Nothing in `GuidedDraft` is persisted. Closing the Sheet discards it, which is FR-007 by
  construction rather than by cleanup.
- `step` is advanced only by a pure reducer over the draft, so every transition is unit-testable —
  the `components/setup/setup-state.ts` precedent, not 042's branch-on-state.
- `bindings` and `apiserver-write` are reachable only *after* a successful save, because 042's
  `RoleAssignmentDraft` needs a saved `issuer_id`. The save is the boundary between "nothing
  written" and "writes offered".
- `origin === 'SeededFromEnvironment'` freezes `config` and skips `review-save`.
