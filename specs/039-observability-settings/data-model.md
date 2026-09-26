# Data Model: Observability Settings

## Persisted (`~/.fathom/integrations.json`, additive)
- `by_context: map<string, ContextIntegrations>` (new)
- `ContextIntegrations { prometheus?: PrometheusConfig, grafana?: GrafanaConfig }`
- `PrometheusConfig { enabled: bool, namespace: string (non-empty), service: string (non-empty), port: u16 (1-65535) }` (unchanged)
- `GrafanaConfig { enabled: bool, base_url: string (http/https URL) }`; legacy `api_key` is read-only for migration and never written
- Legacy top-level `context`, `prometheus`, `grafana`, `argo`, `flux` remain readable.

## Credential store
- Entry `fathom.grafana` / `<context>` holds the API key. Absent means key not set.

## Protocol
- `ObservabilitySettings { context: string, prometheus?: { namespace, service, port }, grafana?: { base_url, key_set: bool } }`. No secret field exists.

## State transitions
- not configured -> configured: Save with valid URL (Grafana) or valid endpoint (Prometheus).
- key not set -> key set: Save with non-blank key; key set -> key not set: `ClearGrafanaKey`.
- Context switch: event for the new context replaces displayed values.
