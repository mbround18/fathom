# Configurable Metrics Polling Interval

## Feature

The metrics refresh interval is now fully configurable via the UI. Users can change how often Prometheus metrics are fetched for a pod (1-300 seconds).

## UI Control

In the **Pod Details** view header, next to the tabs (Details/Logs/Exec), there's now a numeric input:

```
[Details] [Logs] [Exec]                    Metrics refresh (sec): [15]
```

## How It Works

### Default Behavior
- Metrics polling starts with **15 second** interval when pod details panel opens
- This is configurable via the UI field

### Changing the Interval
1. Click the numeric input field showing the current interval (default `15`)
2. Type a new value (1-300 seconds)
3. Press Enter or click away
4. Polling **automatically restarts** with the new interval
5. Changes apply immediately to the active pod

### Constraints
- **Minimum**: 1 second (for frequent updates)
- **Maximum**: 300 seconds (5 minutes, for low-frequency monitoring)
- **Invalid input**: Silently ignored, keeps previous value

## Implementation Details

### State Management
```rust
pub struct PodsView {
    // ...
    pub pod_metrics_interval_secs: u64,  // Current polling interval
}
```

### UI Control
```rust
ui.label("Metrics refresh (sec):");
let mut interval_str = self.pod_metrics_interval_secs.to_string();
ui.text_edit_singleline(&mut interval_str);

if let Ok(parsed) = interval_str.parse::<u64>() {
    if parsed > 0 && parsed <= 300 {
        self.pod_metrics_interval_secs = parsed;
        
        // Restart polling with new interval
        if self.pod_metrics_polling_started {
            actions.push(ViewAction::SendCommand(BackendCmd::StopMetricsPolling));
            actions.push(ViewAction::SendCommand(BackendCmd::StartMetricsPolling {
                namespace: ns.clone(),
                name: name.clone(),
                interval_secs: self.pod_metrics_interval_secs,
            }));
        }
    }
}
```

### Command Flow
```
User edits interval field
    ↓
Value parsed (1-300 range check)
    ↓
If polling active:
    Stop current polling task
    ↓
    Start new polling with new interval
    ↓
Backend restarts metrics polling loop
```

## Use Cases

### Heavy Monitoring (1-5 sec)
- Actively debugging performance issues
- Watching for sudden spikes
- Development environment

### Normal Monitoring (10-30 sec)
- Production pods with steady metrics
- Default 15 sec is good here
- Balances visibility with API load

### Light Monitoring (60-300 sec)
- Long-running stable services
- Minimizing Prometheus query load
- Batch job monitoring

## UI Layout

```
┌─ Pod Details ──────────────────────────────────────────┐
│ namespace / pod-name                                    │
├────────────────────────────────────────────────────────┤
│ [Details] [Logs] [Exec]    Metrics refresh (sec): [15] │
├────────────────────────────────────────────────────────┤
│ (Details tab content with metrics)                      │
│                                                         │
│ 📊 Prometheus Metrics                                  │
│ CPU: 45.2%                                             │
│ Memory: 256.5 MB / 512.0 MB                            │
│ Network RX: 1.25 MB                                    │
│ Network TX: 0.85 MB                                    │
│ Last updated: 2025-01-01T12:00:00Z                     │
└────────────────────────────────────────────────────────┘
```

## Notes

- Changing interval while metrics are fetching: Change takes effect after current fetch completes
- Very rapid interval changes: May cause brief delays as polling restarts
- Interval persists: Per-pod, resets when closing pod details
- Network impact: Lower intervals = more Prometheus queries (monitor your API load)
