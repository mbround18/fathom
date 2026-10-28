# Contract: signed mode, overrides and the host ledger (`crates/keel-trust`)

`keel-trust` is pure Rust with no I/O. The server uses it for the early check and the review;
`keel-executor` uses it as the enforcing check (research R19, R20). Both must reach the same
verdict from the same inputs; a shared test vector suite proves it.

## Trust files on the host (`/etc/keel-runner/trust/`)

Root-owned, mode `0640`, group `keel-trust` (members: `keel-runner`, `keel-exec`). Written only by
the base profile (contracts/host-install.md).

| File | Content |
|---|---|
| `host_ca.pub` | Keel's SSH host CA, one or two keys during rotation. |
| `signers` | Trusted signers: SSH `allowed_signers` lines (principal `keel`, namespace `git`), followed by ASCII-armored OpenPGP public keys. |
| `approvers.json` | `[{ label, credential_id, public_key_cose, alg }]`: approver passkeys. |
| `policy.toml` | `origin` (Keel's web origin), `rp_id`, `require_signed`, `allowed_profiles`, `override_quorum`, `max_override_age` (default `24h`, upper bound on `expires_at - now`), `inventory_name`. |

## Commit check

Input: the Git object store, `base` (from the ledger, or none), `target`, the signers.

1. `target` must exist with a complete object graph. Otherwise `refused (pack_incomplete)`.
2. If `base` is set and `target` is not `base` and not a descendant of `base`: `downgrade`. Only an
   override can authorize it.
3. Walk every commit reachable from `target` and not from `base`, merges included.
4. For each commit, read the signature header (`gpgsig`, or `gpgsig-sha256`):
   - OpenPGP: verify with rpgp over the commit without the header. The issuing key or subkey must
     be in `signers`, have signing capability, and be neither expired nor revoked at the
     committer time.
   - SSH (`-----BEGIN SSH SIGNATURE-----`): namespace `git`, key in `signers`.
   - Missing, unparseable, unknown key or bad signature: the commit is `unsigned`, with that
     detail.
5. Verdict: `signed { signers }` if every commit passes; otherwise `needs_override { failures }`.

`base = none` (first run): the verdict is always `needs_override`, satisfied only by a
`baseline` approval.

## Override challenge

```
challenge = SHA-256( canonical_json({
  "v": "keel-override-v1",
  "kind": "override" | "baseline",
  "workspace": "<uuid>", "binding": "<uuid>", "run": "<uuid>", "job": "<name>",
  "base": "<commit hex>" | null, "target": "<commit hex>",
  "hosts": ["<inventory name>", ...],          // sorted, deduplicated
  "expires_at": "<RFC 3339 UTC, seconds>"
}) )
```

`canonical_json` is RFC 8785 (JCS). The WebAuthn `challenge` is these 32 bytes.

## Override verification on the host

For each approval in `Approvals`:

1. Rebuild the challenge from the host's own `base` (ledger), the `target` it verified, and the
   run, job, workspace, binding and `expires_at` from `Assign` and the approval. The host's
   `inventory_name` must be in `hosts`.
2. `credential_id` must be in `approvers.json`.
3. `client_data_json`: `type == "webauthn.get"`, `challenge` equals the rebuilt challenge
   (base64url), `origin == policy.origin`.
4. `authenticator_data`: `rpIdHash == SHA-256(rp_id)`; flags UP and UV set.
5. The signature (ES256 or EdDSA, per `alg`) verifies over
   `authenticator_data ‖ SHA-256(client_data_json)`.
6. `now < expires_at <= now + max_override_age`.

The override holds when the number of distinct approvers that pass is at least
`max(policy.override_quorum, quorum from Assign)`. Anything else: `refused (bad_override)` with
the first failing step. Verification uses `web_passkeys::verify_assertion`, which is pure and has
no server state.

## Ledger (`/var/lib/keel-executor/ledger.jsonl`)

One JSON line per event, owner `keel-exec`, mode `0600`.

```json
{"seq":42,"at":"…","run":"…","job":"web-deploy","event":"start","base":"…","target":"…",
 "auth":{"kind":"signed","signers":["SHA256:…"]},"prev":"<hex>","hash":"<hex>"}
{"seq":43,"at":"…","run":"…","job":"web-deploy","event":"end","outcome":"succeeded","exit":0,
 "prev":"<hex>","hash":"<hex>"}
```

- `hash = SHA-256(prev ‖ JCS(line without hash))`. The first line's `prev` is 32 zero bytes.
- `auth.kind` is `signed`, `override` (approval ids and approver labels) or `baseline`.
- A job's **base** is the `target` of its latest `end` line with outcome `succeeded` or `failed`.
  A failed run still moves the base, because the host has partly applied it.
- The executor verifies the whole chain at start. A broken chain makes it refuse all work with
  `ledger_corrupt`.
- A `run` that already has a `start` line is refused as `replay`.

## Test vectors

`crates/keel-trust/tests/vectors/` holds repositories and approvals with expected verdicts:
signed, unsigned tip, unsigned commit under a signed merge, unknown key, expired PGP subkey,
downgrade, baseline, override for another host, wrong origin, UV missing, expired, quorum short,
replay. Both the server's early check and the executor run them.
