# Live validation: 035-cloud-auth-plugins (T025, Linux part)

Run 2026-09-26 on Linux (Pop!_OS 24.04). The real backend ran headless via `examples/live_driver.rs`, launched with a GUI-like environment: `PATH=/usr/bin:/bin`, no `AWS_*` variables, and an isolated `HOME`.

**Clusters.** No EKS, GKE or AKS cluster is available. The production context authenticates with a client certificate, and the scoped agent kubeconfig uses a static token, so neither exercises an exec plugin and the real-cloud manual checks (quickstart 1–3 against real clusters) could not be run. Instead, every exec context targets the real API server of the throwaway kind cluster `fathom-fed-1`. The plugins used were:
- fake `kubelogin` and generic plugins that return a 4-hour ServiceAccount token as an `ExecCredential` (1 h expiry) and log each spawn;
- the real `aws` CLI (`/usr/local/bin/aws`) for the EKS context.

| Check | Result |
|-------|--------|
| Detection (FR-002) | Classification for the five test contexts. `CloudAuthDetected` itself spawned nothing. **Pass.** <ul><li>`cap-aks`: aks, resolved at `~/.local/bin/kubelogin` although `~/.local/bin` is not on PATH (GUI-launch search dirs).</li><li>`cap-eks`: eks, at `/usr/local/bin/aws`.</li><li>`cap-gke`: gke, not installed, hint `gcloud components install gke-gcloud-auth-plugin`.</li><li>`cap-generic`: genericExec, not allowed.</li><li>`cap-static`: none.</li></ul> |
| pluginMissing (GKE) | Status `Error`, `plugin_error.kind = pluginMissing`, with provider, command and install hint. The message says a GUI-launched app may not see the shell PATH. **Pass.** |
| Real `aws` without credentials (EKS) | `credentialExpired`, with the verbatim `aws` output (`exit code 253 ... NoCredentials: Unable to locate credentials`) and the hint `Run aws sso login ...`. **Pass** for the failure path. A signed-in EKS connect was not possible. |
| pluginBlocked, then allowAll | The unknown absolute-path command was blocked (`allowKnownOnly`, not run). After `SetExecPolicy allowAll` it was `Connected` with exactly 1 spawn. **Pass.** |
| GUI PATH resolution (quickstart 4) | `cap-aks` connected using `~/.local/bin/kubelogin` with `PATH=/usr/bin:/bin`. **Pass.** |
| Credential cache (FR-006) | 1 spawn per exec context, from the startup reachability probe; detection itself spawned nothing. After that: <ul><li>switching to `cap-aks` spawned nothing (cache hit);</li><li>95s of fleet polling across 5 contexts gave 25 fleet summaries and **0** plugin spawns.</li></ul> **Pass.** |
| Leak check | The fixture token appears in none of the event stream, the backend stderr log or `~/.fathom`. **Pass.** |

**Still open:**
- real EKS, GKE and AKS sign-in (quickstart manual 1–3);
- Windows 11 and macOS 26;
- Finder or Start-menu launch.
