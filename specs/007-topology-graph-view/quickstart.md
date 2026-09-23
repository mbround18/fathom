# Quickstart: Topology Graph View

## Prerequisites

```bash
cd ui && pnpm install   # picks up the new d3-force dependency
```

## Run against the fake backend (no real cluster needed)

```bash
pnpm -C ui dev:fake
```

Open the app, widen the window past tablet width (or use the phone/tablet tab strip), and select
the **Topology** dock tab (laptop+) or **Topology** stack tab (phone/tablet).

## Validate User Story 1 (namespace overview)

1. With the fake backend's default fixture (several namespaces, some with Applications/Services),
   open the Topology pane.
2. **Expected**: one node per namespace; edges only from namespaces that actually contain an
   Application/Helm release or Service; an empty namespace (e.g. `kube-node-lease`) still renders
   as a node with no edges.
3. Trigger the fake backend's "namespace created" scripted event.
4. **Expected**: the new namespace's node appears without reloading the view.

## Validate User Story 2 (continuous zoom)

1. From the overview, scroll/pinch-zoom into one namespace's node.
2. **Expected**: that namespace's workload nodes progressively appear; other namespaces stay
   collapsed to their summary node.
3. Zoom further into one workload node.
4. **Expected**: that workload's pod nodes appear, each showing its phase.
5. Zoom back out fully.
6. **Expected**: view returns to the namespace overview, same pane instance (no route/reload).

## Validate User Story 3 (click-through to inspector)

1. Zoom to pod level, click a pod node.
2. **Expected**: the existing Inspector pane opens on that pod (Details/YAML/Events tabs).
3. Navigate back (pane stack "Back" on phone, or switch back to the Topology tab/slot on wider
   widths).
4. **Expected**: the Topology pane shows the same zoom/pan position as before the click.

## Validate User Story 4 (live updates)

1. Zoom to a Deployment at `N/N` ready replicas.
2. Trigger the fake backend's scripted pod-failure event for one of that Deployment's pods.
3. **Expected**: the pod node's color/status updates in place and the Deployment's ready-count
   label updates, with no new command issued solely by the graph (check via the fake backend's
   "commands received" assertion helper already used by other e2e specs).

## Validate idle CPU (SC-006, manual — not automatable in CI)

1. Open the Topology pane, let the layout settle, and do not touch the mouse/trackpad.
2. Open your OS/browser performance monitor (e.g. Chrome DevTools' Performance Monitor, or
   `top`/Activity Monitor for the whole app).
3. **Expected**: CPU usage for the tab/process drops back to idle within a second or two of the
   layout settling, and stays there. `TopologyCanvas.tsx`'s redraw effect has no
   `requestAnimationFrame` loop or polling interval — it only redraws when the graph, camera, or
   canvas size actually changes (code review note at the effect itself), so an idle graph should
   draw once and then do nothing.

## Automated coverage

```bash
cargo test                                        # ToolKind round-trip (contracts/topology-graph-module.md)
pnpm -C ui test -- topology                        # unit tests: build.ts, layout.ts, status.ts
pnpm -C ui e2e -- us1-topology us2-topology us3-topology us4-topology
```
