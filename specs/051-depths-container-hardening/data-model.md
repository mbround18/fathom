# Data Model: Depths container hardening

- **Level**: `ok | root | escape`; stack level = max of its containers (`rollup`).
- **Reason**: `{ code, severity (info|root|escape), detail }`; `code` is a stable kebab-case id (e.g. `privileged`, `runtime-socket`, `host-pid`, `cap-sys-admin`, `seccomp-unconfined`, `root-user`, `sensitive-mount`).
- **Profile**: `{ level, reasons[] }` per container; `deciding()` returns the reasons that set the level.
- **DaemonMode**: `rootful | rootless | userns | unknown` (unknown treated as rootful).
- **ShellPolicy**: `strict` (default) | `relaxed`. Decision table:

| level  | strict            | relaxed           |
|--------|-------------------|-------------------|
| ok     | role              | role              |
| root   | admins only       | role              |
| escape | refused for all   | admins only       |

- **LintFinding**: `{ service, code, severity, detail, line? }`; same codes as Reason plus `no-user`.
- **ComposePolicy**: `warn` (default) | `enforce`. Under enforce, escape findings block save/up unless an admin sends `acknowledge: true`; that is audited. Root findings never block.
- **Audit events**: `shell.open {container, level, policy}`, `shell.refused {container, level, reason}`, `compose.acknowledged {stack, codes[]}`.
