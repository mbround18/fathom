# Contract: cloud auth protocol additions

Additive to protocol v1 (no version bump). Serde round-trip test required for every variant.

| Item | Direction | Notes |
|---|---|---|
| `CloudAuthDetected { infos }` | event | On Initialize/RefreshContexts; no plugin spawned |
| `SetExecPolicy { context, policy }` | command | Persists per-context policy; re-probes context |
| `ContextStatusUpdated.pluginError` | event | Optional, alongside existing error message |
| `FleetContextSummary.pluginError` | event | Optional; distinguishes plugin failure from 401/403 |

Guarantees: no field carries a token, env value, or unredacted stderr. Errors are verbatim
(Principle I) except `[redacted]` markers.
