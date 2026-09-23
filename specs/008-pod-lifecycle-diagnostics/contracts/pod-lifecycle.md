# Contract: Pod Lifecycle Diagnostics

No new `BackendCmd` or `BackendEvent` variants. This feature reuses the existing `SelectPod` /
`PodDetailsUpdated` read pair (`pods.tsx`'s `read()` already sends it) and the existing
`RefreshEvents` / `EventsUpdated` pair. The only protocol change is an additive field on the
existing `PodDetailsUpdated` payload's `PodDetails` type.

## Command/Event flow (unchanged)

```text
UI: descriptor.read(ref)
  -> SelectPod { namespace, name }          (existing)
  -> RefreshEvents                          (existing, already sent today)
  -> RefreshRelatedResources { ... }        (existing)

Backend:
  fetch_pod_details(...)
    -> BackendEvent::ResourceRead(ResourceReadEvent::PodDetailsUpdated(PodDetails { ..., containers_lifecycle }))
                                             (PodDetails now carries containers_lifecycle — new)
  refresh_events(...)  [pre-existing handler, untouched]
    -> BackendEvent::ResourceList(ResourceListEvent::EventsUpdated(EventsSummary))
                                             (unchanged; the UI's eventsFor() selector filters
                                              this by involved_object_kind == "Pod" and name/ns)
```

## `PodDetailsUpdated` payload (delta)

```jsonc
{
  "type": "PodDetailsUpdated",
  "data": {
    // ...all existing PodDetails fields, unchanged...
    "containers_lifecycle": [
      {
        "name": "init-migrate",
        "is_init": true,
        "image": "registry.example.com/migrate:1.0",
        "ready": false,
        "state": { "phase": "terminated", "reason": "Completed", "exit_code": 0,
                    "started_at": "2026-09-20T08:15:01Z", "finished_at": "2026-09-20T08:15:04Z" },
        "restart_count": 0,
        "last_state": null,
        "probes": [],
        "hooks": []
      },
      {
        "name": "api",
        "is_init": false,
        "image": "registry.example.com/api:1.4.2",
        "ready": false,
        "state": { "phase": "running", "reason": null, "exit_code": null,
                    "started_at": "2026-09-20T08:15:05Z", "finished_at": null },
        "restart_count": 3,
        "last_state": { "phase": "terminated", "reason": "OOMKilled", "exit_code": 137,
                          "started_at": "2026-09-20T08:14:00Z", "finished_at": "2026-09-20T08:14:59Z" },
        "probes": [
          { "probe_type": "readiness", "mechanism": "httpGet", "target": "GET /healthz:8080",
            "period_seconds": 10, "failure_threshold": 3, "success_threshold": 1 }
        ],
        "hooks": [
          { "hook_type": "preStop", "mechanism": "exec", "target": "exec: /bin/sh -c sleep 5" }
        ]
      }
    ]
  }
}
```

## Verification

- Round-trip serde test for `ContainerLifecycle`, `ContainerState`, `ProbeSpec`,
  `LifecycleHookSpec`, and the extended `PodDetails` in `tests/protocol_serde.rs`.
- `cargo test --test export_bindings` regenerates
  `ui/src/protocol/generated/{ContainerLifecycle,ContainerState,ProbeSpec,LifecycleHookSpec,PodDetails}.ts`;
  committed output is checked for drift in CI (existing mechanism, unchanged).
- Frontend consumes the new field only through `PodDetails['containers_lifecycle']`
  (`ui/src/protocol/types` re-export), never a hand-maintained duplicate type.
