/**
 * 036 cytoscape: visual topology graph. Default export so it can be `React.lazy`-loaded (the
 * cytoscape chunk is only fetched at md+ when the daemon feeds a topology). The accessible grouped
 * list stays the text alternative and the small-width fallback.
 */
import cytoscape, { type Core, type StylesheetJson } from 'cytoscape'
import { useEffect, useMemo, useRef } from 'react'

import type { TopologyEdge, TopologyNode } from '@/protocol/types'

import {
  buildModel,
  diffModels,
  emptyModel,
  RELATION_STYLE,
  type GraphModel,
  type GraphNode,
} from './graphModel'
import { paletteFor, type GraphPalette } from './graphTheme'

interface Props {
  nodes: Record<string, TopologyNode>
  edges: readonly TopologyEdge[]
  selected: string | null
  onSelect: (uid: string) => void
}

const toEle = (n: GraphNode) => ({
  group: 'nodes' as const,
  data: { id: n.id, label: n.label, kind: n.kind, health: n.health, shape: n.shape },
  position: { x: n.x, y: n.y },
})

function styles(p: GraphPalette, labels: boolean): StylesheetJson {
  const ring = (c: string) => ({ 'border-color': c })
  const rel = Object.entries(RELATION_STYLE).map(([k, s]) => ({
    selector: `edge[relation = "${k}"]`,
    style: {
      'line-style': s.lineStyle,
      width: s.width,
      'target-arrow-shape': s.arrow,
    } as never,
  }))
  return [
    {
      selector: 'node',
      style: {
        shape: 'data(shape)' as never,
        width: 22,
        height: 22,
        'background-color': p.fallback,
        'border-width': 3,
        'border-color': p.unknown,
        label: labels ? 'data(label)' : '',
        color: p.fg,
        'font-size': 10,
        'text-valign': 'bottom',
        'text-margin-y': 4,
        'text-max-width': '120px',
        'text-wrap': 'ellipsis',
        'min-zoomed-font-size': 7,
      },
    },
    ...Object.entries(p.kinds).map(([k, c]) => ({
      selector: `node[kind = "${k}"]`,
      style: { 'background-color': c },
    })),
    { selector: 'node[health = "healthy"]', style: ring(p.healthy) },
    { selector: 'node[health = "degraded"]', style: ring(p.degraded) },
    { selector: 'node[health = "unhealthy"]', style: { ...ring(p.unhealthy), 'border-width': 4 } },
    {
      selector: 'edge',
      style: {
        'curve-style': 'haystack',
        'line-color': p.edge,
        'target-arrow-color': p.edge,
        opacity: 0.7,
      },
    },
    ...rel,
    {
      selector: 'node:selected',
      style: {
        width: 32,
        height: 32,
        'overlay-color': p.accent,
        'overlay-opacity': 0.25,
        'overlay-padding': 6,
        'underlay-color': p.accent,
        'underlay-opacity': 0.35,
        'underlay-padding': 8,
        label: 'data(label)',
        'font-weight': 700,
        'z-index': 10,
      },
    },
  ]
}

function apply(cy: Core, prev: GraphModel, next: GraphModel): void {
  const d = diffModels(prev, next)
  cy.batch(() => {
    for (const id of d.removeEdges) cy.getElementById(id).remove()
    for (const id of d.removeNodes) cy.getElementById(id).remove()
    if (d.addNodes.length > 0) cy.add(d.addNodes.map(toEle))
    for (const n of d.updateNodes)
      cy.getElementById(n.id).data({ health: n.health, label: n.label })
    if (d.addEdges.length > 0)
      cy.add(
        d.addEdges.map((e) => ({
          group: 'edges' as const,
          data: { id: e.id, source: e.source, target: e.target, relation: e.relation },
        })),
      )
    if (d.relayout) for (const n of next.nodes.values()) cy.getElementById(n.id).position(n)
  })
  if (d.relayout) cy.fit(undefined, 24)
}

export default function CytoscapeGraph({ nodes, edges, selected, onSelect }: Props) {
  const host = useRef<HTMLDivElement>(null)
  const cyRef = useRef<Core | null>(null)
  const model = useRef<GraphModel>(emptyModel())
  const onSelectRef = useRef(onSelect)
  useEffect(() => {
    onSelectRef.current = onSelect
  }, [onSelect])
  const next = useMemo(() => buildModel(nodes, edges, selected), [nodes, edges, selected])

  useEffect(() => {
    if (!host.current) return
    const isDark = document.documentElement.classList.contains('dark')
    const cy = cytoscape({
      container: host.current,
      elements: [],
      style: styles(paletteFor(isDark), true),
      minZoom: 0.05,
      maxZoom: 3,
      wheelSensitivity: 0.3,
      textureOnViewport: true,
      hideEdgesOnViewport: true,
      pixelRatio: 1,
    })
    cy.on('tap', 'node', (ev: cytoscape.EventObject) => {
      onSelectRef.current((ev.target as cytoscape.NodeSingular).id())
    })
    cyRef.current = cy
    return () => {
      cy.destroy()
      cyRef.current = null
      model.current = emptyModel()
    }
  }, [])

  useEffect(() => {
    const cy = cyRef.current
    if (!cy) return
    if (next.labels !== model.current.labels) {
      const isDark = document.documentElement.classList.contains('dark')
      cy.style(styles(paletteFor(isDark), next.labels))
    }
    apply(cy, model.current, next)
    model.current = next
  }, [next])

  useEffect(() => {
    const cy = cyRef.current
    if (!cy) return
    cy.$(':selected').unselect()
    if (selected) cy.getElementById(selected).select()
  }, [selected, next])

  return (
    <div className="relative h-72 w-full border-b border-border bg-background md:h-full md:min-h-72">
      <div
        ref={host}
        className="h-full w-full"
        role="img"
        aria-label="Topology graph. The grouped list of objects is the accessible equivalent."
        data-testid="daemon-topology-graph"
        data-node-count={next.nodes.size}
        data-hidden-count={next.hidden}
      />
      {next.hidden > 0 && (
        <p className="absolute bottom-1 left-2 rounded bg-background/80 px-1 text-xs text-muted-foreground">
          Showing {next.nodes.size} of {next.nodes.size + next.hidden}, unhealthy first
        </p>
      )}
    </div>
  )
}
