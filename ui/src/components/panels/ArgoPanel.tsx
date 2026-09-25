/**
 * Argo CD pane. No Argo-specific backend exists, so this reads the generic CRD catalog
 * (`CrdCatalogUpdated`) for argoproj.io Application/ApplicationSet/AppProject and lists instances
 * via the existing `RefreshCrdInstances` path. Read-only; rows open the CR in the custom-resource
 * YAML inspector.
 */
import { useEffect } from 'react'

import { ARGO_GROUP, detectArgo, parseArgoRows } from '@/features/argo/argo'
import { cn } from '@/lib/utils'
import { useStore } from '@/store'

import { Empty, Loading } from '../shell/PaneStates'

const TONE: Record<string, string> = {
  Synced: 'text-emerald-500',
  Healthy: 'text-emerald-500',
  OutOfSync: 'text-amber-500',
  Progressing: 'text-amber-500',
  Degraded: 'text-red-500',
  Missing: 'text-red-500',
}

export default function ArgoPanel() {
  const catalog = useStore((s) => s.crds.catalog)
  const loaded = useStore((s) => s.crds.catalogLoaded)
  const instances = useStore((s) => s.crds.instances)
  const send = useStore((s) => s.send)
  const selectCrdKind = useStore((s) => s.selectCrdKind)
  const navigate = useStore((s) => s.navigate)
  const { found, missing } = detectArgo(catalog)
  const foundKey = found.map((f) => f.kind).join(',')

  useEffect(() => {
    for (const e of found) {
      send({
        type: 'RefreshCrdInstances',
        data: { group: e.group, kind: e.kind, version: e.versions[0] ?? '', namespace: null },
      })
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [foundKey, send])

  if (!loaded) return <Loading rows={4} label="Checking for Argo CD" />
  if (found.length === 0) {
    return (
      <div data-testid="argo-not-detected">
        <Empty
          title="Argo CD not detected on this cluster"
          detail={`Checked the CRD catalog for ${ARGO_GROUP}: ${missing.join(', ')}. None are installed.`}
        />
      </div>
    )
  }

  const rows = found.flatMap((e) => {
    const st = instances[`${e.group}/${e.kind}`]
    return st ? parseArgoRows(e.kind, st.items).map((r) => ({ r, e })) : []
  })
  const denied = found.filter((e) => e.permission_denied).map((e) => e.kind)

  return (
    <div data-testid="argo-panel" className="flex flex-col gap-2 p-2">
      {missing.length > 0 && (
        <p className="text-xs text-muted-foreground">Not installed: {missing.join(', ')}.</p>
      )}
      {denied.length > 0 && (
        <p className="text-xs text-muted-foreground">
          Permission denied listing: {denied.join(', ')}.
        </p>
      )}
      {rows.length === 0 ? (
        <p data-testid="argo-empty" className="text-sm text-muted-foreground">
          Argo CD is installed; no Argo resources found.
        </p>
      ) : (
        <div className="overflow-x-auto">
          <table data-testid="argo-table" className="w-full text-sm">
            <thead className="border-b border-border text-left text-xs text-muted-foreground uppercase">
              <tr>
                <th className="px-2 py-1 font-medium">Kind</th>
                <th className="px-2 py-1 font-medium">Name</th>
                <th className="hidden px-2 py-1 font-medium sm:table-cell">Namespace</th>
                <th className="px-2 py-1 font-medium">Sync</th>
                <th className="px-2 py-1 font-medium">Health</th>
              </tr>
            </thead>
            <tbody>
              {rows.map(({ r, e }) => (
                <tr
                  key={`${r.kind}/${r.namespace ?? ''}/${r.name}`}
                  className="border-b border-border/50"
                >
                  <td className="px-2 py-1 text-xs">{r.kind}</td>
                  <td className="px-2 py-1 font-mono">
                    <button
                      type="button"
                      data-testid="argo-row"
                      className="text-left underline-offset-2 hover:underline focus-visible:underline"
                      aria-label={`Open ${r.kind} ${r.name} YAML`}
                      onClick={() => {
                        selectCrdKind(e)
                        navigate('custom-resources')
                      }}
                    >
                      {r.name}
                    </button>
                  </td>
                  <td className="hidden px-2 py-1 font-mono text-muted-foreground sm:table-cell">
                    {r.namespace ?? '—'}
                  </td>
                  <td className={cn('px-2 py-1', TONE[r.sync])}>{r.sync}</td>
                  <td className={cn('px-2 py-1', TONE[r.health])}>{r.health}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </div>
  )
}
