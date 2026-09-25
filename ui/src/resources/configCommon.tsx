/** Shared helpers for the batch D descriptors (HPA, PriorityClass, RuntimeClass, Lease, webhooks). */
import { formatAge } from './format'
import type { DetailSection } from './types'

export function parseJson(raw: string): Record<string, unknown> {
  try {
    const v: unknown = JSON.parse(raw)
    return v && typeof v === 'object' && !Array.isArray(v) ? (v as Record<string, unknown>) : {}
  } catch {
    return {}
  }
}

export const obj = (v: unknown): Record<string, unknown> =>
  v && typeof v === 'object' && !Array.isArray(v) ? (v as Record<string, unknown>) : {}
export const arr = (v: unknown): unknown[] => (Array.isArray(v) ? v : [])
export const str = (v: unknown): string => (typeof v === 'string' ? v : '')

interface Meta {
  name: string
  creation_timestamp: string
  labels: Record<string, string>
  annotations: Record<string, string>
  namespace?: string
}

export function metadataSection<R extends { name: string; namespace?: string }, D extends Meta>(
  namespaced: boolean,
): DetailSection<R, D> {
  return {
    id: 'metadata',
    title: 'Metadata',
    render: ({ row, detail }) => {
      if (!row && !detail) return null
      const fields = [{ label: 'Name', value: detail?.name ?? row?.name ?? '', mono: true }]
      if (namespaced) {
        fields.push({
          label: 'Namespace',
          value: detail?.namespace ?? row?.namespace ?? '',
          mono: true,
        })
      }
      if (detail) {
        fields.push({
          label: 'Created',
          value: `${detail.creation_timestamp} (${formatAge(detail.creation_timestamp)} ago)`,
          mono: true,
        })
      }
      return { type: 'fields', fields }
    },
  }
}

export function labelSections<R, D extends Meta>(): DetailSection<R, D>[] {
  return [
    {
      id: 'labels',
      title: 'Labels',
      render: ({ detail }) => (detail ? { type: 'map', entries: detail.labels } : null),
    },
    {
      id: 'annotations',
      title: 'Annotations',
      render: ({ detail }) => (detail ? { type: 'map', entries: detail.annotations } : null),
    },
  ]
}
