# Research: 035-cloud-auth-plugins

## R1. Who spawns the plugin?
- **Decision**: Fathom owns plugin invocation (a `cloud_auth` runner) and feeds the resulting
  token into the kube `Config` (exec entry replaced by a cached token/refresh layer).
- **Why**: kube-rs `Client::try_from(Config)` re-runs exec per fresh client, defeating any cache
  across polls; its errors are generic (`Error::Auth`). Owning invocation gives timeout,
  allowlist, single-flight, typed errors.
- **Alternatives**: (a) rely on kube-rs alone: no cross-client cache, weak errors. (b) keep one
  long-lived `Client` per context in fleet: larger refactor, overlaps P2 federation; noted as a
  future option. (c) vendor AWS/GCP/Azure SDK auth: rejected by the MVP design constraint.
- **Open verification (T004)**: confirm the exact kube 2.0.1 mechanism for injecting an externally
  obtained credential (`AuthInfo.token` vs a custom tower auth layer with refresh).

## R2. Cache design
- In-memory `HashMap<Identity, Entry>` behind `tokio::sync::Mutex`; single-flight per key.
  Identity = hash(command, args, env, apiVersion, cluster server+CA when provideClusterInfo).
  Expiry from `status.expirationTimestamp` minus 60s skew; default TTL 5m when absent.
- Token held in a secret wrapper with redacting `Debug`, zeroized on drop.
- Fleet cost: 30s poll x N contexts. EKS tokens last ~15m, GKE/AKS ~1h, so spawns drop ~30x to ~120x.

## R3. Detection
- Classify by command basename (strip `.exe/.cmd`): `aws` with args `eks get-token` or
  `aws-iam-authenticator` -> eks; `gke-gcloud-auth-plugin` -> gke; `kubelogin` -> aks.
  Resolve binary without spawning, using PATH plus augmented GUI PATH.

## R4. Security
- Threat: an untrusted/shared kubeconfig runs an arbitrary command. Mitigation: known-command
  allowlist, per-context opt-in for others visible in UI; argv only, never `sh -c`.
- Env: kubeconfig-specified env plus inherited env (AWS_PROFILE etc. needed); never log env values.
- Sandbox: Tauri does not sandbox the Rust backend; spawning is backend-only and never exposed to
  the webview. MCP reuses the same context + policy and cannot trigger arbitrary exec.
- Limits: 20s timeout, kill on drop, stdout cap 64 KiB, stderr cap 4 KiB, no stdin, no TTY.

## R5. Errors
- `io::ErrorKind::NotFound` -> pluginMissing; non-zero exit -> pluginFailed (stderr heuristics
  for expired/SSO/login -> credentialExpired); elapsed -> pluginTimeout; interactiveMode Always
  -> unsupported. Redact JWT / `k8s-aws-v1.` / `ya29.` shapes.

## R6. Platform
- Windows: PATHEXT resolution, `CREATE_NO_WINDOW`. macOS/Linux: GUI PATH augmentation.
  Tests use a cross-platform fake plugin (small Rust test binary, not shell scripts).

## R7. Dependencies
- Prefer none new; `which`/`secrecy` only if already transitive, otherwise justify in plan.

## Implementation notes (T004, T024)
- T004 result: kube 2.0.1 sends `Authorization: Bearer` when `Config.auth_info.token`
  (`SecretString`) is set and `auth_info.exec` is `None`; client cert/key data are likewise
  injectable. Fathom therefore runs the plugin itself (shared cache) and injects the result, so
  kube-rs never spawns it. Verified against a fake ExecCredential binary, not a live cloud.
- T024 dependencies: no new crates. `secrecy` 0.10 was already a direct dependency (038). A
  `which` crate was not added: resolution is a small hand-rolled PATH/PATHEXT search that must
  also honour the augmented GUI PATH and never spawn, which `which` does not simplify.
- Data-model deviations: `ContextStatusUpdated` and `FleetContextSummary` gained
  `plugin_error: Option<PluginError>`; `CloudAuthInfo` gained `exec_policy`.
