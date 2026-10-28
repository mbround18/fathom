# Contract: registry requests (spec 061)

This is the exact set of HTTP requests an update check may make. Anything not listed here is a
bug. Tests assert this list against the fake registry's request log.

## Reference resolution

| Watched reference | Registry API base | Repository |
|---|---|---|
| `mbround18/keel:latest` | `https://registry-1.docker.io` | `mbround18/keel` |
| `docker.io/library/foo` | `https://registry-1.docker.io` | `library/foo` |
| `ghcr.io/org/keel:edge` | `https://ghcr.io` | `org/keel` |
| `registry.example:5000/keel` | `https://registry.example:5000` | `keel` |

`docker.io` and `index.docker.io` map to `registry-1.docker.io`. Nothing else is aliased.

## Requests

`Accept` for manifest requests, sent exactly as:

```text
application/vnd.oci.image.index.v1+json,
application/vnd.docker.distribution.manifest.list.v2+json,
application/vnd.oci.image.manifest.v1+json,
application/vnd.docker.distribution.manifest.v2+json
```

`User-Agent: keel/<version or revision> (update-check)`.

| # | Request | When | Counts as a pull on Docker Hub |
|---|---|---|---|
| 1 | `HEAD /v2/<repo>/manifests/<tag>` without a token | Every check, unless a cached token is valid | No |
| 2 | `GET <realm>?service=<service>&scope=repository:<repo>:pull` | After a 401 from #1 or #3 carrying `Bearer` | No |
| 3 | `HEAD /v2/<repo>/manifests/<tag>` with `Authorization: Bearer <token>` | After #2 | No |
| 4 | `GET /v2/<repo>/manifests/<digest>` | The remote digest differs from the running one and isn't cached | Yes (with #5, one pull) |
| 5 | `GET /v2/<repo>/manifests/<platform digest>` | #4 returned an index or manifest list | Counted together with #4 |
| 6 | `GET /v2/<repo>/blobs/<config digest>` | After #4 or #5 | No |

So a check costs:

- **Unchanged, or already cached**: 1–3 requests, 0 pulls.
- **New digest**: 4–6 requests, 1 pull, once per process per digest.

## Rules

1. **Redirects**:
   - Blob GETs may redirect (Docker Hub sends blobs to a CDN). At most 3 redirects are followed,
     `https` only, and the `Authorization` header is dropped on a cross-host redirect.
   - Manifest requests do not follow redirects.
2. **Realm**: `https` only. Basic credentials go only to the realm, and only when configured. The
   realm host may differ from the registry host (Docker Hub uses `auth.docker.io`).
3. **Body limits**: manifests and indexes up to 4 MiB, config blobs up to 4 MiB, token responses
   up to 64 KiB. A larger body is `bad_response`.
4. **Digest checks**: the bodies of #4, #5 and #6 are hashed. A digest mismatch is
   `bad_response`.
5. **Index entry selection**:
   - Choose the entry with `platform.os == "linux"` and `platform.architecture` equal to Keel's
     (`amd64` or `arm64`), plus a matching `variant` when the entry has one.
   - Skip entries whose platform is `unknown/unknown`, or whose annotations include
     `vnd.docker.reference.type`.
   - With no match, the outcome is `cant_tell` with the reason `no_image_for_platform`.
6. **Timeouts**: 10 s per request, 30 s per check.
7. **Error codes**:

   | Code | Meaning |
   |---|---|
   | `auth_failed` | 401 or 403 after a token |
   | `not_found` | 404 on #1 or #3 |
   | `rate_limited` | 429, or Docker Hub's `TOOMANYREQUESTS` |
   | `timeout` | A request or check timeout was reached |
   | `network` | Connection or TLS failure |
   | `bad_response` | Anything malformed, oversized or mismatched |

8. **Logging**: the method, host, path and status of each request at `debug`. Never headers,
   never the token, never credentials.

## Fake registry for tests

`apps/keel/server/tests/support/fake_registry.rs` is an axum router on `127.0.0.1:0`, with:

- `/token`, which requires `Basic` when configured;
- manifests and blobs from in-memory fixtures;
- a request log.

The fixtures cover:

- a single-manifest image;
- an index with amd64, arm64 and an attestation entry;
- an index without Keel's platform;
- a config without labels;
- a 429;
- a body with a bad digest.

The registry client is constructed with the fake's `http://127.0.0.1:<port>` base and realm. This
constructor is test-only; configuration never yields `http`.
