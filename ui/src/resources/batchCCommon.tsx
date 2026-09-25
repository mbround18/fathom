/** Shared helpers for the Batch C (storage + config) descriptors. */
import type { BackendCmd } from '@/protocol/types'

import { deleteAction } from './actions'
import { formatAge } from './format'
import { compareText } from './search'
import type { ActionDescriptor, ColumnDef, DetailSection, ObjectRef } from './types'

interface Meta {
  name: string
  namespace?: string
  creation_timestamp: string
  labels: Record<string, string>
  annotations: Record<string, string>
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

export const nameColumn = <R extends { name: string }>(): ColumnDef<R> => ({
  id: 'name',
  header: 'Name',
  priority: 1,
  minWidth: 150,
  grow: 3,
  cell: (r) => r.name,
  text: (r) => r.name,
  sort: (a, b) => compareText(a.name, b.name),
})

export const namespaceColumn = <R extends { namespace: string }>(): ColumnDef<R> => ({
  id: 'namespace',
  header: 'Namespace',
  priority: 2,
  minWidth: 112,
  cell: (r) => r.namespace,
  text: (r) => r.namespace,
  sort: (a, b) => compareText(a.namespace, b.namespace),
})

/** Namespaced delete with the RBAC capability gate (`accessCheck`) and the shared confirm dialog. */
export function gatedDelete<R extends { name: string; namespace: string }>(
  make: (namespace: string, name: string) => BackendCmd,
  resource: string,
  group?: string,
): ActionDescriptor<R> {
  return {
    ...deleteAction<R>(make),
    accessCheck: (row) => ({ verb: 'delete', resource, group, namespace: row.namespace }),
  }
}

/** Cluster-scoped delete (no namespace in the command). */
export function gatedClusterDelete<R extends { name: string }>(
  make: (name: string) => BackendCmd,
  resource: string,
  group?: string,
): ActionDescriptor<R> {
  return {
    id: 'delete',
    label: 'Delete',
    done: 'Deleted',
    doing: 'Deleting',
    prompt: 'confirm',
    destructive: true,
    bulk: true,
    accessCheck: () => ({ verb: 'delete', resource, group, namespace: null }),
    plan: (rows) => rows.map((r) => ({ cmd: make(r.name), rows: [r] })),
  }
}

export const clusterRef =
  (section: string, kind: string) =>
  (r: { name: string }): ObjectRef => ({ section, kind, namespace: null, name: r.name })

/** Parse a Kubernetes quantity to a comparable number (cpu cores, bytes, or a count). */
export function parseQuantity(q: string): number | undefined {
  const m = /^([0-9.]+)([a-zA-Z]*)$/.exec(q.trim())
  if (!m) return undefined
  const n = Number(m[1])
  if (!Number.isFinite(n)) return undefined
  const mult: Record<string, number> = {
    '': 1,
    m: 1e-3,
    k: 1e3,
    K: 1e3,
    M: 1e6,
    G: 1e9,
    T: 1e12,
    Ki: 1024,
    Mi: 1024 ** 2,
    Gi: 1024 ** 3,
    Ti: 1024 ** 4,
  }
  const f = mult[m[2] ?? '']
  return f === undefined ? undefined : n * f
}
