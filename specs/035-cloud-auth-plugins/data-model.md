# Data Model: 035-cloud-auth-plugins

Protocol (Rust source of truth in `src/protocol/v1/`, TS generated, additive):

- `CloudProvider`: `eks | gke | aks | genericExec | none`
- `CloudAuthInfo { context, provider, command?, installed?, resolvedPath?, execPolicyAllowed, installHint? }`
- `PluginErrorKind`: `pluginMissing | pluginFailed | pluginTimeout | credentialExpired | pluginBlocked | pluginInteractiveUnsupported`
- `PluginError { kind, context, provider, command, message (redacted, truncated), hint? }`
- `ExecPolicy`: `allowKnownOnly` (default) | `allowAll`, per context, persisted additively in
  `~/.fathom/` config (Principle VI: absent field = default).
- Events: `SystemEvent::CloudAuthDetected { infos }`; existing `ContextStatusUpdated` and the fleet
  context summary gain an optional `pluginError`.
- Command: `SetExecPolicy { context, policy }`.

Backend-internal (never in protocol):
- `CredentialCacheEntry { identity, token (secret), expires_at?, fetched_at }`, in-memory only,
  dropped for removed contexts on kubeconfig reload.

Validation: `command` non-empty; hints static per provider; message <= 4 KiB.
