# Contract: migration bundle (`.keelx`)

An uncompressed POSIX tar. Each entry is JSON, sorted, with canonical key order.

```
manifest.json            # format: 1, source_instance_id, source_version, created_at, created_by,
                         # entries: [{ path, sha256, bytes }], signature (Ed25519 over the canonical manifest
                         # without the signature field), source_public_key
workspaces/<slug>.json   # workspace, projects, approval policies, scope mappings
bindings/<id>.json       # url, branch, transport, known_host, poll_interval, allow_force_push, credential *name*
inventories/<name>/meta.json
inventories/<name>/revisions/<seq>.json   # every revision: content, hash, source commit, author, status
credentials/<name>.json  # name, kind, fingerprint, external_ref?  (no values, no sealed bytes)
teams/<name>.json        # local teams and grants (with profile scopes)
profiles.json            # profiles per workspace (assignments are not exported)
signers.json             # trusted signers per workspace (public keys only)
runs/<id>.json           # optional history: run row, approvals, host results (no logs)
```

## Not exported

Receiver secrets, credential values, sessions, passkeys, API tokens, SSH CA keys, hosts and
their revisions, and audit rows. Hosts enrol again on the destination, which has its own CAs; the
host ledgers are the record of what they ran.
The destination audits the import itself, and the source's audit stays at the source as its
record.

## Import checks

All checks run before anything is written.

1. Every entry's hash matches.
2. The signature verifies.
3. The source key is in `trusted_instances`. If it is not, the plan asks to trust the shown
   fingerprint, which needs step-up.
4. `format` is supported.
5. References are closed: every binding's credential name, every job's inventory, and every
   grant's team exist in the bundle or at the destination.
6. Conflicts are listed: a slug, inventory name or host identity that exists with different
   content. Each one needs `skip` or `rename:<new>`.

## Apply

Apply is one transaction. Imported revisions get new local ids, with `imported_from` set to
`source_instance_id/revision_id`. The `migrations` row records the bundle hash, both instance ids
and the choices made, and is audited (SC-007).
