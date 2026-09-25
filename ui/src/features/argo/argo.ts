/** Pure helpers for the Argo CD pane: CRD detection and sync/health extraction from raw CR JSON. */
import type { CrdCatalogEntry } from '@/protocol/types'

export const ARGO_GROUP = 'argoproj.io'
export const ARGO_KINDS = ['Application', 'ApplicationSet', 'AppProject'] as const
export type ArgoKind = (typeof ARGO_KINDS)[number]

export function detectArgo(catalog: readonly CrdCatalogEntry[]): {
  found: CrdCatalogEntry[]
  missing: ArgoKind[]
} {
  const found = ARGO_KINDS.map((k) =>
    catalog.find((c) => c.group === ARGO_GROUP && c.kind === k),
  ).filter((c): c is CrdCatalogEntry => c !== undefined)
  const have = new Set(found.map((c) => c.kind))
  return { found, missing: ARGO_KINDS.filter((k) => !have.has(k)) }
}

export interface ArgoRow {
  kind: string
  namespace: string | null
  name: string
  sync: string
  health: string
}

export function parseArgoRows(kind: string, items: readonly string[]): ArgoRow[] {
  const out: ArgoRow[] = []
  for (const raw of items) {
    try {
      const o = JSON.parse(raw) as {
        metadata?: { name?: string; namespace?: string }
        status?: { sync?: { status?: string }; health?: { status?: string } }
      }
      if (!o.metadata?.name) continue
      out.push({
        kind,
        namespace: o.metadata.namespace ?? null,
        name: o.metadata.name,
        sync: o.status?.sync?.status ?? '—',
        health: o.status?.health?.status ?? '—',
      })
    } catch {
      // skip malformed entry
    }
  }
  return out
}
