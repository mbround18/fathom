/**
 * Small canvas showing one Route's traffic split (028-knative-pane). Data comes from
 * `buildKnativeTrafficGraph`; an accessible text legend carries the same percentages.
 */
import { useEffect, useMemo, useRef } from 'react'

import type { KnativeOverview } from '@/protocol/types'
import { buildKnativeTrafficGraph } from '@/topology/knative'

const W = 360
const H = 160

function color(style: CSSStyleDeclaration, v: string, fallback: string): string {
  const raw = style.getPropertyValue(v).trim()
  return raw ? `oklch(${raw})` : fallback
}

export function KnativeTrafficGraph({
  overview,
  namespace,
  name,
}: {
  overview: KnativeOverview
  namespace: string
  name: string
}) {
  const graph = useMemo(
    () => buildKnativeTrafficGraph(overview, { namespace, name }),
    [overview, namespace, name],
  )
  const ref = useRef<HTMLCanvasElement>(null)

  useEffect(() => {
    const canvas = ref.current
    const g = canvas?.getContext('2d')
    if (!canvas || !g) return
    const style = getComputedStyle(canvas)
    g.clearRect(0, 0, W, H)
    const pos = (id: string) => {
      const n = graph.nodes.find((x) => x.id === id)
      return { x: 40 + (n?.x ?? 0), y: H / 2 + (n?.y ?? 0) }
    }
    const fg = color(style, '--foreground', '#eee')
    g.font = '10px sans-serif'
    g.textAlign = 'center'
    for (const e of graph.edges) {
      const a = pos(e.source)
      const b = pos(e.target)
      g.strokeStyle = color(style, '--muted-foreground', '#888')
      g.beginPath()
      g.moveTo(a.x, a.y)
      g.lineTo(b.x, b.y)
      g.stroke()
      if (e.label) {
        g.fillStyle = fg
        g.fillText(e.label, (a.x + b.x) / 2, (a.y + b.y) / 2 - 4)
      }
    }
    for (const n of graph.nodes) {
      const p = pos(n.id)
      const tone = {
        ok: '--status-ok',
        warn: '--status-warn',
        err: '--status-err',
        muted: '--muted-foreground',
      }[n.tone]
      g.fillStyle = color(style, tone, '#888')
      g.beginPath()
      g.arc(p.x, p.y, 8, 0, Math.PI * 2)
      g.fill()
      g.fillStyle = fg
      g.fillText(n.label, p.x, p.y + 20, 140)
    }
  }, [graph])

  if (graph.nodes.length === 0) return null
  return (
    <div data-testid="knative-traffic-graph" className="flex flex-col gap-1">
      <canvas ref={ref} width={W} height={H} className="max-w-full" aria-hidden="true" />
      <ul className="font-mono text-xs">
        {graph.edges.map((e) => {
          const target = graph.nodes.find((n) => n.id === e.target)
          return (
            <li key={e.id} data-testid="knative-traffic-target">
              {target?.label} {e.label}
              {target?.detail ? ` — ${target.detail}` : ''}
            </li>
          )
        })}
      </ul>
    </div>
  )
}
