# Logs Component Refactoring

## Overview

Extracted the logs streaming functionality from the pod-specific implementation into a reusable, generic `LogViewer` component with pub/sub-style messaging. This enables logs viewing across any resource type (pods, deployments, daemonsets, etc.) while maintaining clean separation of concerns.

## Architecture

### Core Components

#### `LogStreamId` (enum)
Identifies the source of a log stream using a type-safe enum. Variants:
- `Pod { namespace, name }`
- `Deployment { namespace, name }` (future use)
- `StatefulSet { namespace, name }` (future use)
- `DaemonSet { namespace, name }` (future use)
- `ReplicaSet { namespace, name }` (future use)

#### `LogStreamState` (struct)
Encapsulates the state for a single log stream:
- `stream_id`: Identifies the source
- `logs`: Bounded vec of log lines (max 1000)
- `container`: Optional selected container
- `autoscroll`, `autoscroll_forced_off`: Auto-scroll behavior state
- `filter`, `filter_case_sensitive`, `filter_include`: Filter config

**Key Methods**:
- `add_lines()`: Append lines to the stream (respects max size)
- `filtered_logs()`: Returns filtered view based on current settings
- `filter_count()`: Returns (filtered count, total count) tuple

#### `LogViewer` (struct)
Stateless rendering component with static methods for UI rendering:

**`render_controls()`**
- Renders: Start, Stop, Clear, Auto-scroll toggle, Save…, Copy, Container selector
- Returns: `(should_start, should_stop, should_clear, container_changed)`
- Enables decoupled backend command triggering

**`render_filter_controls()`**
- Renders: Filter input, Clear Filter, Match case toggle, With/Without mode, counter
- Directly mutates `LogStreamState` for filter changes

**`render_logs()`**
- Renders scrollable area with filtered logs (monospace font)
- Manages auto-scroll behavior: detects scroll position, disables on manual scroll up, re-enables at bottom
- Returns `ScrollAreaOutput` for advanced scroll manipulation if needed

**`render_full()`**
- Convenience method calling all three render functions sequentially
- Returns control flags for easy integration

## Usage Pattern in Views

### In Pods View

```rust
// Initialize on pod selection
if self.pod_logs_stream.is_none() {
    let stream_id = LogStreamId::Pod { namespace: ns.clone(), name };
    self.pod_logs_stream = Some(LogStreamState::new(stream_id));
}

// Render logs tab
if let Some(stream) = &mut self.pod_logs_stream {
    let containers = self.pod_details.as_ref()
        .map(|d| d.containers.clone())
        .unwrap_or_default();
    
    let (should_start, should_stop, _, _) = 
        LogViewer::render_full(ui, stream, &containers);
    
    if should_start {
        actions.push(ViewAction::SendCommand(
            BackendCmd::StartPodLogs { 
                namespace: ns.clone(),
                name: name.clone(),
                container: stream.container.clone(),
            }
        ));
    }
}

// Handle streaming events
if let BackendEvent::Stream(StreamEvent::PodLogsChunk { lines, .. }) = event {
    if let Some(stream) = &mut self.pod_logs_stream {
        stream.add_lines(lines.clone());
    }
}
```

## Future Extensions

### Adding Logs to Other Resources

To add logs viewing to deployments, daemonsets, etc.:

1. Add a `deployment_logs_stream: Option<LogStreamState>` field to the view struct
2. Initialize with `LogStreamId::Deployment { namespace, name }` on selection
3. Use `LogViewer::render_full()` identically to pods
4. Wire the backend command to match the resource type
5. Handle the same `PodLogsChunk` event (or enhance protocol to be generic)

### Protocol Enhancement (Optional)

For true genericity, could enhance the protocol:

```rust
pub enum StreamEvent {
    LogsChunk { 
        source: LogStreamId,  // Replace stream_id with source
        lines: Vec<String>,
    },
    // ...
}
```

This would require updating the backend to emit `LogsChunk` with the source identifier, but would enable a single event handler for all resource types.

## Benefits

- **Separation of Concerns**: Logs UI logic isolated from resource-specific views
- **Reusability**: Copy-paste free implementation across pods, deployments, etc.
- **Testability**: Pure data structures and stateless rendering make unit testing straightforward
- **Maintainability**: Single source of truth for logs UI behavior (auto-scroll, filter, etc.)
- **Extensibility**: Easy to add new resource types or filter modes

## Implementation Notes

- `LogStreamState` is cloned when passed to `LogViewer` methods; no shared state required
- Filter operations are performed on-demand during rendering for efficiency
- Auto-scroll state manages two flags to distinguish user-initiated disable from auto-disable
- Container dropdown uses `ComboBox::from_id_salt()` with stream ID to ensure unique widget IDs across multiple streams
