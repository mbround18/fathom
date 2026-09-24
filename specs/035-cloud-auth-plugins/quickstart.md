# Quickstart / Validation: 035-cloud-auth-plugins

Automated (CI), using a fake plugin binary that emits ExecCredential JSON and counts calls:
1. Missing plugin: exec `command: gke-gcloud-auth-plugin`, empty PATH -> UI shows pluginMissing with provider, context, install hint.
2. Cache: 5 fleet polls inside token lifetime -> counter == 1; past expiry -> 2.
3. 401 with a cached token -> exactly one re-invocation.
4. Hanging plugin -> pluginTimeout (test uses shortened timeout).
5. Unknown command -> pluginBlocked; after SetExecPolicy allowAll -> connects.
6. Leak test: fixture token absent from logs, events, exports.

Manual on Tier 1 OSes (Ubuntu 26.04, Windows 11, macOS 26) against real clusters:
1. EKS: `aws sso login`, select context, connects; log out, see credentialExpired hint.
2. GKE: with `gke-gcloud-auth-plugin` installed, connects; rename it, see pluginMissing.
3. AKS: `kubelogin convert-kubeconfig -l azurecli`; connects; in Fleet, confirm no per-poll spawn.
4. Launch from Finder/Start menu (no shell PATH) and confirm the plugin is found.
