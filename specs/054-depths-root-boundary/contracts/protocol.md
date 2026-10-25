# Contract: protocol changes (054, `crates/depths-protocol`)

All new variants:
- get serialisation round-trip tests;
- get ts-rs bindings, regenerated and drift-checked;
- bump the protocol version, because `Confirm`, `OpenShell` and `Hello` gain fields. Those fields
  are optional on the wire, so older clients still parse.

## Types

```rust
/// WebAuthn options/credentials as JSON (webauthn-rs serde shapes; base64url binary fields).
pub struct PasskeyRequest { pub ceremony: CeremonyId, pub options: serde_json::Value }
pub struct PasskeyResponse { pub ceremony: CeremonyId, pub credential: serde_json::Value }

pub struct PasskeyInfo { pub id: String, pub name: String, pub created: Timestamp,
                         pub last_used: Option<Timestamp> }

pub struct Verification { pub verified: bool, pub at: Option<Timestamp>,
                          pub window_secs: u32, pub enrolled: bool, pub required: bool }

pub struct GuardRefusal { pub rule: String, pub setting: String,
                          pub service: Option<String> }

pub struct GuardException { pub service: String, pub image: Option<String>,
                            pub grants: Vec<GuardGrant>, pub note: Option<String>,
                            pub granted: Option<String> }
pub struct GuardGrant { pub kind: GuardGrantKind /* device|bind|cap|namespace */,
                        pub value: String, pub read_only: Option<bool> }
```

## Cmd

| Command | Reply | Notes |
|---|---|---|
| `GetVerification` | `Verification` | Also pushed after sign-in and after any change. |
| `PasskeyBegin { purpose: Register { name } \| Verify }` | `PasskeyChallenge(PasskeyRequest)` | `Register` needs no step-up for the first passkey. Later ones need a fresh one. |
| `PasskeyFinish(PasskeyResponse)` | `PasskeyRegistered(PasskeyInfo)` \| `Verification` \| `PasskeyRefused { reason }` | The ceremony is single-use and session-bound. |
| `ListPasskeys` | `Passkeys { items }` | Own passkeys only. |
| `PreviewPasskeyRemove { id }` | `Preview` (with `step_up`) | Refused when this is the last passkey. |
| `RecoverPasskeys { code }` | `Verification` \| `PasskeyRefused { reason: recovery_invalid }` | Removes the person's passkeys, and allows one `Register` without a prior assertion. |
| `GetGuardExceptions { stack }` | `GuardExceptions { stack, items }` | Read-only, proxied from the guard. An empty list when the guard has none. |

### Changed commands

```rust
Confirm { preview_id, acknowledge_findings, #[serde(default)] assertion: Option<PasskeyResponse> }
OpenShell { container, shell, cols, rows, #[serde(default)] assertion: Option<PasskeyResponse> }
```

## Event

- `Preview` gains `step_up: Option<PasskeyRequest>`, which is set when confirming will need a
  fresh check.
- New refusals, as variants of the existing refusal event:
  - `StepUpRequired { request: PasskeyRequest }`;
  - `PasskeyEnrolmentRequired`;
  - `PasskeyRefused { reason: replayed | wrong_origin | unknown_credential | expired | cancelled | recovery_invalid }`;
  - `RuntimeGuardUnreachable`.
- `OpFinished` gains `guard: Option<GuardRefusal>`.
- `FileRefused` gains the variant `Guard(GuardRefusal)`.

## Gates (server)

The command order is: authenticate → trust (052) → **verification** → role/stack access →
**step-up** → handler.

- **Verification:** an admin or owner without `verified_at` is given the restricted access set.
  The handler sees `Role::User` with no stacks, as for 052's untrusted networks.
- **Step-up:** for the risky commands in research R11, there must be either `verified_at` within
  the window or a valid `assertion`.
  - A valid assertion sets `verified_at` to now.
  - A failed or missing one returns the refusal and changes nothing (FR-019), and is audited.

## Helper `Hello` (053 contract)

`Hello { proto: 1, openat2, uid, gid, build: String }`. The server refuses with
`helper_unavailable { reason: build_mismatch }` when `build` differs from its own `DEPTHS_BUILD_ID`.
