# Feature Specification: Observability Works Out of the Box

**Feature Branch**: `041-observability-autoconnect`

**Created**: 2026-09-26

**Status**: Implemented

**Input**: "Three nav items should work out of the box on a cluster running Prometheus, metrics-server,
Argo CD and Grafana (kind `fathom-dev`): Topology renders mostly blank; Metrics needs the Prometheus
endpoint typed in by hand even though the stack scan finds it; Grafana needs a manual `kubectl
port-forward`."

## User Scenarios & Testing

### User Story 1 - Topology shows the namespace graph (P1)

Opening Topology from the Tools menu shows the graph of the active namespace, fitted to the pane
and labelled, on phone, laptop and wide layouts.

**Acceptance Scenarios**:

1. **Given** an active namespace, **When** Topology opens, **Then** the graph shows that namespace's
   objects and edges, with node labels, zoomed to fit the pane.
2. **Given** the laptop or wide layout, **When** Topology opens in the tools sheet, **Then** the sheet
   is wide enough for the graph (not a ~310px strip).
3. **Given** no active namespace ("all namespaces"), **Then** the graph covers every namespace.
4. **Given** the user has panned or zoomed, **When** the pane resizes or data refreshes, **Then** their
   camera is kept; **Fit** re-fits on demand.
5. Node colours come from the theme tokens (which are complete `oklch(...)` values) and never fall
   back to black.

### User Story 2 - Prometheus connects without input (P1)

With nothing saved for the context, Fathom uses the Prometheus the stack scan found, reached through
the Kubernetes API server's service proxy (`/api/v1/namespaces/{ns}/services/{svc}:{port}/proxy`),
so no port-forward and no user input is needed.

**Acceptance Scenarios**:

1. **Given** a detected Prometheus and nothing saved, **When** the cluster connects or the Metrics pane
   opens, **Then** metrics queries succeed and the pane reports "Auto-detected ns/svc:port via the
   API server proxy".
2. **Given** saved settings (the manual override), **Then** they are tried first; if they do not answer,
   the detected Prometheus is used and the status says so.
3. **Given** "Test connection", **Then** the typed namespace/service/port are probed through the proxy
   (previously only the port was used, against localhost).
4. Detection never persists settings, and a non-Prometheus component (e.g. Alertmanager) is never
   used as Prometheus.
5. A failed resolution is cached for 60s per context so polling does not rescan every tick.
6. If the proxy is not reachable (e.g. RBAC denies `services/proxy`), the pre-041 port-forward
   fallback still runs.

### User Story 3 - One-click Grafana (P1)

A detected in-cluster Grafana gets an **Open Grafana** button. It starts a Fathom port-forward to the
Service (visible, stoppable and tagged with the Service under Port forwards), saves
`http://localhost:<port>` as the Grafana base URL (keeping any saved API key) and opens it in the
browser. Grafana denies framing by default, so it is opened, not embedded.

**Acceptance Scenarios**:

1. **Given** a detected Grafana, **When** the user clicks Open Grafana, **Then** `StartServicePortForward
   {namespace, service, port}` is sent; the backend picks a pod behind the Service, maps the Service
   port to its targetPort (numeric or named), binds the first free local port from 10000 up (the
   Service port itself when it is already ≥ 10000 and free; feedback 2026-09-27 #27) and answers
   with `ServicePortForwardReady`.
2. **Given** an active forward for the same pod and port, **Then** it is reused, not duplicated.
3. **Given** no pod behind the Service, **Then** the command fails and the pane shows the error.

## Requirements

- **FR-001**: `BackendCmd::StartServicePortForward` is app-local (like `StartPortForward`), audited as a
  `PortForwardChange` on the Service.
- **FR-002**: `SystemEvent::ServicePortForwardReady {namespace, service, port, local_port, tunnel_id}`.
- **FR-003**: `PrometheusClient` and all Prometheus query paths (pod/workload metrics, history, disk
  usage, KEDA) go through one `PromEndpoint` (URL or service proxy), with a 10s timeout.
- **FR-004**: The per-state `prometheus_client` field is removed (it leaked across context switches).

## Tests

- Rust units: candidate order and dedup, Prometheus-only detection, float timestamps, scientific
  values, proxy path, targetPort mapping, local port choice.
- UI units: `tokenColor`, `fitCamera`.
- Fake e2e: `metrics-grafana.spec.ts` (Open Grafana happy path and failure), `nav-walk.spec.ts`.
- Live (kind `fathom-dev`): `ui/tests/live/observability-autoconnect.spec.ts`.
