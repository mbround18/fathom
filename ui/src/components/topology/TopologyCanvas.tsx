/**
 * The topology graph canvas (US1-US4). Builds the graph from the store's already-cached lists
 * (`buildTopologyGraph`), lays it out (`layoutTopologyGraph`), and hand-draws it on a `<canvas>`
 * rather than an SVG-per-node tree (data-model.md, spec.md Assumptions — thousands of pods must
 * stay responsive). Pan/zoom and the camera's own scale (research.md R3) drive which tiers are
 * "in focus" and therefore fetched and rendered; `PaneState.topology` persists the camera across
 * re-layout (research.md R4).
 */
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'

import { descriptorFor } from '@/resources'
import { applications } from '@/resources/applications'
import { detailKey, listKey, useStore, type PaneId } from '@/store'
import { buildTopologyGraph, WORKLOAD_KINDS } from '@/topology/build'
import { NODE_RADIUS, layoutTopologyGraph, snapshotPositions } from '@/topology/layout'
import { NEUTRAL_TONE } from '@/topology/status'
import { nodeId, type TopologyGraph, type TopologyNode } from '@/topology/types'
import { deriveFocus, NAMESPACE_FOCUS_SCALE } from '@/topology/viewport'

import {
  clampScale,
  hitTest,
  screenToWorld,
  worldToScreen,
  zoomAt,
  type Camera,
} from './TopologyCanvas.utils'

const TONE_VAR: Record<TopologyNode['tone'], string> = {
  ok: '--status-ok',
  warn: '--status-warn',
  err: '--status-err',
  muted: '--muted-foreground',
}

function toneColor(style: CSSStyleDeclaration, tone: TopologyNode['tone']): string {
  const v = style.getPropertyValue(TONE_VAR[tone]).trim()
  return v ? `oklch(${v})` : '#888'
}

export function TopologyCanvas({ paneId }: { paneId: PaneId }) {
  const containerRef = useRef<HTMLDivElement>(null)
  const canvasRef = useRef<HTMLCanvasElement>(null)
  const [size, setSize] = useState({ width: 800, height: 600 })
  // The layout must be centered on the real canvas box, not the 800x600 placeholder used until
  // the ResizeObserver first reports (on ultra-wide docks the graph otherwise sits far from the
  // canvas center, so zooming around the center pushes it off screen).
  const [measured, setMeasured] = useState(false)

  const pane = useStore((s) => s.panes[paneId])
  const updatePane = useStore((s) => s.updatePane)
  const openInspector = useStore((s) => s.openInspector)
  const send = useStore((s) => s.send)
  const ctx = useStore((s) => s.session.activeContext)
  const namespaces = useStore((s) => s.session.namespaces)
  const lists = useStore((s) => s.lists)
  const details = useStore((s) => s.details)

  const listKeyFor = useCallback(
    (section: string) =>
      listKey(
        ctx,
        section,
        descriptorFor(section)?.namespaced === false ? 'Cluster' : 'Namespaced',
        null,
      ),
    [ctx],
  )
  const detailKeyFor = useCallback(
    (kind: string, namespace: string | null, name: string) => detailKey(ctx, kind, namespace, name),
    [ctx],
  )

  // Layout runs in the same coordinate box as the canvas (`layoutTopologyGraph`'s `forceCenter`
  // targets `size.width/2, size.height/2`), so the identity camera (`scale: 1, x: 0, y: 0`)
  // already centers the graph on first paint — no extra translation needed.
  const [camera, setCamera] = useState<Camera>(() => pane?.topology ?? { scale: 1, x: 0, y: 0 })

  // ---- responsive size (ResizeObserver, no continuous polling)
  useEffect(() => {
    const el = containerRef.current
    if (!el) return
    const ro = new ResizeObserver((entries) => {
      const box = entries[0]?.contentRect
      if (box) {
        setSize({ width: Math.max(1, box.width), height: Math.max(1, box.height) })
        setMeasured(true)
      }
    })
    ro.observe(el)
    return () => {
      ro.disconnect()
    }
  }, [])

  // ---- namespace focus is a pure function of scale + the namespace list (no dependency on the
  // heavier workload/pod tiers), so it can be derived before the first graph build.
  const namespaceFocus = useMemo(
    () =>
      deriveFocus({
        scale: camera.scale,
        namespaceNodes: namespaces.map((n) => ({ label: n })),
        workloadNodes: [],
        namespaceIdToName: new Map(),
      }).namespaceFocus,
    [camera.scale, namespaces],
  )

  // ---- pass 1: namespace/app/helm/node tiers + workload tier for focused namespaces, no pods
  // yet, so workload focus can be derived from *this* build's actual workload nodes.
  const pass1 = useMemo(
    () =>
      buildTopologyGraph({
        namespaces,
        lists,
        details,
        viewport: { namespaceFocus, workloadFocus: new Set() },
        listKeyFor,
        detailKeyFor,
      }),
    [namespaces, lists, details, namespaceFocus, listKeyFor, detailKeyFor],
  )

  const namespaceIdToName = useMemo(
    () => new Map(namespaces.map((n) => [nodeId('Namespace', null, n), n])),
    [namespaces],
  )

  const workloadFocus = useMemo(
    () =>
      deriveFocus({
        scale: camera.scale,
        namespaceNodes: namespaces.map((n) => ({ label: n })),
        workloadNodes: pass1.nodes.filter((n) => n.tier === 'workload'),
        namespaceIdToName,
      }).workloadFocus,
    [camera.scale, namespaces, pass1.nodes, namespaceIdToName],
  )

  // ---- pass 2: adds the pod tier for focused workloads.
  const graph = useMemo(
    () =>
      workloadFocus.size === 0
        ? pass1
        : buildTopologyGraph({
            namespaces,
            lists,
            details,
            viewport: { namespaceFocus, workloadFocus },
            listKeyFor,
            detailKeyFor,
          }),
    [pass1, workloadFocus, namespaces, lists, details, namespaceFocus, listKeyFor, detailKeyFor],
  )

  // ---- layout: re-run whenever the node/edge set changes, seeded from the prior run's
  // positions so newly-focused nodes fade in near their neighbors instead of jumping in cold.
  // This is an effect (not a `useMemo`) because it both reads and writes `priorPositions.current`
  // — a ref must not be touched during render (react-hooks/refs).
  const priorPositions = useRef<Map<string, { x: number; y: number }>>(new Map())
  const [laidOut, setLaidOut] = useState<TopologyGraph>(graph)
  useEffect(() => {
    const out = layoutTopologyGraph(graph, priorPositions.current, size)
    priorPositions.current = snapshotPositions(out)
    setLaidOut(out)
    // `size` intentionally excluded: a resize alone should not re-run the (expensive-ish)
    // simulation, only a graph change (or the first real measurement) should.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [graph, measured])

  // ---- lazy fetch on entering focus (T032): namespace focus -> RefreshRelatedResources per
  // Application in that namespace (for the Application -> workload edge); workload focus ->
  // the workload's own `select` command (for its pod list).
  const fetchedNamespaces = useRef(new Set<string>())
  useEffect(() => {
    const appRows = (lists[listKeyFor(applications.section)]?.rows ?? []) as unknown as {
      namespace: string
      instance: string
    }[]
    for (const ns of namespaceFocus) {
      if (fetchedNamespaces.current.has(ns)) continue
      fetchedNamespaces.current.add(ns)
      for (const a of appRows.filter((r) => r.namespace === ns)) {
        send({
          type: 'RefreshRelatedResources',
          data: {
            section: applications.section,
            kind: 'Application',
            namespace: a.namespace,
            name: a.instance,
          },
        })
      }
    }
  }, [namespaceFocus, lists, listKeyFor, send])

  const fetchedWorkloads = useRef(new Set<string>())
  useEffect(() => {
    for (const key of workloadFocus) {
      if (fetchedWorkloads.current.has(key)) continue
      fetchedWorkloads.current.add(key)
      const [ns, kind, ...rest] = key.split('/')
      const name = rest.join('/')
      const wk = WORKLOAD_KINDS.find((w) => w.kind === kind)
      if (wk && ns && name) {
        for (const cmd of wk.descriptor.read?.({
          section: wk.descriptor.section,
          kind: wk.kind,
          namespace: ns,
          name,
        }) ?? []) {
          send(cmd)
        }
      }
    }
  }, [workloadFocus, send])

  // ---- persist the camera on pane state (debounced via rAF), mirroring how list panes persist
  // scrollTop (research.md R4).
  const persistFrame = useRef(0)
  const persistCamera = useCallback(
    (next: Camera) => {
      cancelAnimationFrame(persistFrame.current)
      persistFrame.current = requestAnimationFrame(() => updatePane(paneId, { topology: next }))
    },
    [paneId, updatePane],
  )

  // ---- draw
  const draw = useCallback(() => {
    const canvas = canvasRef.current
    const container = containerRef.current
    if (!canvas || !container) return
    const dpr = window.devicePixelRatio || 1
    canvas.width = size.width * dpr
    canvas.height = size.height * dpr
    const g = canvas.getContext('2d')
    if (!g) return
    g.setTransform(dpr, 0, 0, dpr, 0, 0)
    const style = getComputedStyle(container)
    g.clearRect(0, 0, size.width, size.height)

    const edgeColor = toneColor(style, NEUTRAL_TONE)
    g.strokeStyle = edgeColor
    g.globalAlpha = 0.5
    g.lineWidth = 1
    const byId = new Map(laidOut.nodes.map((n) => [n.id, n]))
    for (const e of laidOut.edges) {
      const a = byId.get(e.source)
      const b = byId.get(e.target)
      if (a?.x === undefined || a.y === undefined || b?.x === undefined || b.y === undefined)
        continue
      const from = worldToScreen(camera, a.x, a.y)
      const to = worldToScreen(camera, b.x, b.y)
      g.beginPath()
      g.moveTo(from.x, from.y)
      g.lineTo(to.x, to.y)
      g.stroke()
      if (e.label) {
        const fg = style.getPropertyValue('--foreground').trim()
        g.globalAlpha = 1
        g.fillStyle = fg ? `oklch(${fg})` : '#eee'
        g.font = '10px sans-serif'
        g.textAlign = 'center'
        g.fillText(e.label, (from.x + to.x) / 2, (from.y + to.y) / 2 - 4)
        g.globalAlpha = 0.5
      }
    }
    g.globalAlpha = 1

    for (const n of laidOut.nodes) {
      if (n.x === undefined || n.y === undefined) continue
      const p = worldToScreen(camera, n.x, n.y)
      const r = NODE_RADIUS[n.tier] * camera.scale
      if (p.x < -r || p.y < -r || p.x > size.width + r || p.y > size.height + r) continue // viewport cull
      g.beginPath()
      g.arc(p.x, p.y, Math.max(2, r), 0, Math.PI * 2)
      g.fillStyle = toneColor(style, n.tone)
      g.globalAlpha = n.loading ? 0.4 : 1
      g.fill()
      g.globalAlpha = 1
      if (camera.scale >= NAMESPACE_FOCUS_SCALE / 2) {
        g.fillStyle = style.getPropertyValue('--foreground').trim()
          ? `oklch(${style.getPropertyValue('--foreground').trim()})`
          : '#eee'
        g.font = '10px sans-serif'
        g.textAlign = 'center'
        g.fillText(n.label, p.x, p.y + r + 12, 120)
      }
    }
  }, [laidOut, camera, size])

  // T060 (idle-CPU, SC-006): this is the entire redraw trigger -- no `requestAnimationFrame`
  // loop, no polling interval. `draw` only changes identity (and therefore only re-runs this
  // effect) when `laidOut`/`camera`/`size` actually change, so an idle graph with a settled
  // layout and a still camera draws exactly once and then does nothing until the next real
  // store update, pointer drag, or wheel event.
  useEffect(() => draw(), [draw])

  // ---- pan/zoom input
  const dragRef = useRef<{ startX: number; startY: number; camX: number; camY: number } | null>(
    null,
  )

  const onWheel = useCallback(
    (e: React.WheelEvent<HTMLCanvasElement>) => {
      e.preventDefault()
      const rect = e.currentTarget.getBoundingClientRect()
      const factor = Math.exp(-e.deltaY * 0.001)
      setCamera((c) => {
        const next = zoomAt(c, factor, e.clientX - rect.left, e.clientY - rect.top)
        persistCamera(next)
        return next
      })
    },
    [persistCamera],
  )

  const onPointerDown = useCallback(
    (e: React.PointerEvent<HTMLCanvasElement>) => {
      e.currentTarget.setPointerCapture(e.pointerId)
      dragRef.current = { startX: e.clientX, startY: e.clientY, camX: camera.x, camY: camera.y }
    },
    [camera.x, camera.y],
  )

  const onPointerMove = useCallback(
    (e: React.PointerEvent<HTMLCanvasElement>) => {
      const drag = dragRef.current
      if (!drag) return
      setCamera((c) => {
        const next: Camera = {
          scale: c.scale,
          x: drag.camX + (e.clientX - drag.startX),
          y: drag.camY + (e.clientY - drag.startY),
        }
        persistCamera(next)
        return next
      })
    },
    [persistCamera],
  )

  const onPointerUp = useCallback((e: React.PointerEvent<HTMLCanvasElement>) => {
    dragRef.current = null
    e.currentTarget.releasePointerCapture(e.pointerId)
  }, [])

  const movedRef = useRef(false)
  const onPointerDownTrack = useCallback(
    (e: React.PointerEvent<HTMLCanvasElement>) => {
      movedRef.current = false
      onPointerDown(e)
    },
    [onPointerDown],
  )
  const onPointerMoveTrack = useCallback(
    (e: React.PointerEvent<HTMLCanvasElement>) => {
      if (dragRef.current) movedRef.current = true
      onPointerMove(e)
    },
    [onPointerMove],
  )

  const onClick = useCallback(
    (e: React.MouseEvent<HTMLCanvasElement>) => {
      if (movedRef.current) return // T041: a drag/pan should not also fire a click-to-inspect.
      const rect = e.currentTarget.getBoundingClientRect()
      const world = screenToWorld(camera, e.clientX - rect.left, e.clientY - rect.top)
      const hit = hitTest(world, laidOut.nodes)
      if (hit) openInspector(hit.ref, paneId)
    },
    [camera, laidOut.nodes, openInspector, paneId],
  )

  return (
    <div
      ref={containerRef}
      className="relative h-full min-h-0 w-full min-w-0"
      data-testid="topology-canvas-host"
    >
      <canvas
        ref={canvasRef}
        data-testid="topology-canvas"
        role="img"
        aria-label="Cluster topology graph"
        style={{ width: size.width, height: size.height, touchAction: 'none' }}
        onWheel={onWheel}
        onPointerDown={onPointerDownTrack}
        onPointerMove={onPointerMoveTrack}
        onPointerUp={onPointerUp}
        onClick={onClick}
      />
      <div className="pointer-events-none absolute right-2 bottom-2 font-mono text-xs text-muted-foreground">
        {Math.round(clampScale(camera.scale) * 100)}%
      </div>
    </div>
  )
}
