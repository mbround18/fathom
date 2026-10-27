# Contract: receivers (`POST /hooks/{receiver_id}`)

This is the public endpoint that source-control hosts call. It is unauthenticated except for the
signature check, rate-limited per receiver and per source IP, and its body is capped at 1 MiB.

## Request

| Provider | Signature | Delivery id | Event accepted |
|---|---|---|---|
| `github` | `X-Hub-Signature-256: sha256=<hex HMAC-SHA256(body)>` | `X-GitHub-Delivery` | `X-GitHub-Event: push` (`ping` answers 200 and is recorded) |
| `gitlab` | `X-Gitlab-Token: <secret>` (constant-time compare) | `X-Gitlab-Event-UUID` | `X-Gitlab-Event: Push Hook` |
| `forgejo` | `X-Forgejo-Signature` or `X-Gitea-Signature: <hex HMAC-SHA256(body)>` | `X-Forgejo-Delivery` / `X-Gitea-Delivery` | `X-Forgejo-Event: push` / `X-Gitea-Event: push` |
| `generic` | `X-Keel-Signature: sha256=<hex>` | `X-Keel-Delivery` (required) | Body `{ "repository": url, "ref": "refs/heads/<branch>" }` |

- The signature is checked against `secret_current`, then against `secret_previous` while
  `previous_expires_at` is in the future.
- From the payload, only the repository clone URLs (`clone_url`, `ssh_url`, `git_http_url`,
  `git_ssh_url`), `ref` and the delivery id are read. Commit data in the payload is ignored
  (research R5).

## Order of checks

Every check is recorded in `deliveries`, and every refusal is audited.

1. The receiver exists and is active. Otherwise reply `404`, with `unknown_receiver` or
   `receiver_disabled`.
2. The signature is valid. Otherwise `401 bad_signature`.
3. The delivery id has not been seen for this receiver. Otherwise `200 {"decision":"duplicate"}`.
4. The ref is a branch. Otherwise `202 ref_not_branch`, recorded and ignored.
5. A bound, active binding matches the normalized URL and branch. Otherwise `403`, with
   `no_matching_binding` or `binding_disabled`.
6. Reply `202 {"decision":"accepted","delivery":"<id>"}`, then fetch asynchronously. The result
   is `synced`, `duplicate`, `stale`, `history_rewritten` or `sync_failed`.

The response never echoes the payload, and never reveals which bindings exist.

## After a sync

`synced` makes the server:
1. read `.keel/jobs.yaml` at the new commit and upsert `jobs`;
2. revise any `repo` inventories whose path changed;
3. for each job whose `paths` filter matches the changed files, create a run (data-model `runs`),
   going through the approval policy.
