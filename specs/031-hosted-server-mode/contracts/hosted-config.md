# Contract: Configuration and cluster requirements

## Environment / flags (`fathom-server`, clap `env`)

`FATHOM_EXTERNAL_URL`, `FATHOM_OIDC_ISSUER`, `FATHOM_OIDC_CLIENT_ID`,
`FATHOM_OIDC_CLIENT_SECRET` (or `_FILE`), `FATHOM_OIDC_CA_FILE`, `FATHOM_OIDC_SCOPES`,
`FATHOM_CLAIM_USERNAME`, `FATHOM_CLAIM_GROUPS`, `FATHOM_USERNAME_PREFIX`, `FATHOM_GROUPS_PREFIX`,
`FATHOM_ACCESS_MODEL=passthrough|impersonation`, `FATHOM_KUBE_TOKEN=id|access`,
`FATHOM_AUDIT_VIEWER_GROUPS`, `FATHOM_LIMIT_*`, `FATHOM_SESSION_ABSOLUTE`, `FATHOM_SESSION_IDLE`,
`FATHOM_STATE_DIR`, `FATHOM_UI_DIR` (default `/ui`), `FATHOM_LISTEN` (default `0.0.0.0:8080`).
Exit codes: 2 = invalid config, 3 = OIDC discovery failed (both print an actionable message, and
no port is bound).

## Helm values (deploy/helm/fathom/values.yaml, schema-validated)

```yaml
externalUrl: https://fathom.example.com     # required
oidc: { issuer: "", clientId: "", clientSecret: "", existingSecret: "", caSecret: "", scopes: [] }
claims: { username: preferred_username, groups: groups, usernamePrefix: "oidc:", groupsPrefix: "oidc:" }
accessModel: passthrough                    # or impersonation
kubeToken: id
ingress: { enabled: false, className: "", tls: [] }
limits: { maxSessions: 50, maxSessionsPerUser: 3, maxWatchesPerUser: 30, maxStreamsPerUser: 10 }
audit: { viewerGroups: [] }
persistence: { enabled: false, size: 1Gi }
image: { repository: ghcr.io/mbround18/fathom-server, tag: "" }
replicaCount: 1                             # schema max 1 in v1
```

## Cluster requirements

- Pass-through: API server started with `--oidc-issuer-url` (same issuer), `--oidc-client-id`
  (= Fathom client id, or an `aud` the ID token carries), matching username/groups claim and
  prefixes. RBAC bindings target `oidc:<username>` / `oidc:<group>`.
- Impersonation: chart-created ClusterRole allows only `impersonate` on `users` and `groups`;
  operator binds real permissions to the mapped names. Fathom SA has no read verbs.
- Fathom's own SA never performs user-visible reads or writes.
