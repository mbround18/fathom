# Contract: Observability Settings (protocol v1)

## Commands
- `GetObservabilitySettings` `{}` : read-only; answers with `ObservabilitySettings` for the active context.
- `ConfigureGrafana` `{ base_url, api_key }` : persists URL for the active context; non-empty `api_key` replaces the stored key, empty keeps it. Emits `ObservabilitySettings`, plus an error notification on persistence failure.
- `ClearGrafanaKey` `{}` : removes the stored key. Emits `ObservabilitySettings`.
- `ConfigurePrometheus` `{ namespace, service, port }` : unchanged; additionally emits `ObservabilitySettings`.

## Event
- `ObservabilitySettings` `{ context, prometheus: {namespace,service,port}|null, grafana: {base_url,key_set}|null }`

## Guarantees
- No event or log line ever carries the API key.
- Settings are keyed by context; an event for context A never populates the form for context B (UI ignores events whose `context` differs from the active one).
