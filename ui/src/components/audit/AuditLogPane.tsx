/**
 * Audit Log viewer (026-mutation-audit-log-viewer): a read-only, newest-first list of the
 * mutating actions this app has completed, across all contexts. The history is local to this
 * machine and bounded; it is not Kubernetes' own audit log.
 */
import { useEffect, useMemo } from 'react'

import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import type { AuditActionKind, AuditLogEntry } from '@/protocol/types'
import { useStore } from '@/store'
import { applyAuditFilter, isDefaultFilter } from '@/store/auditLogFilter'

import { Empty } from '../shell/PaneStates'

const ACTION_LABELS: Record<AuditActionKind, string> = {
  yamlApply: 'Apply YAML',
  helmUpgrade: 'Helm Upgrade',
  helmRollback: 'Helm Rollback',
  helmUninstall: 'Helm Uninstall',
  nodeCordon: 'Node Cordon',
  nodeUncordon: 'Node Uncordon',
  nodeDrain: 'Node Drain',
  nodeTaint: 'Node Taint',
  delete: 'Delete',
  bulkDelete: 'Bulk Delete',
  scale: 'Scale',
  restart: 'Restart',
  cronJobControl: 'CronJob Control',
  portForwardChange: 'Port Forward Change',
  settingsChange: 'Settings Change',
  teamStoreChange: 'Team Store Change',
  agentAction: 'Agent Action',
  mcpToggle: 'MCP Server Toggle',
}

const SELECT_CLASS =
  'h-[var(--touch-target)] rounded-md border border-input bg-transparent px-2 text-sm'

function describeTarget(e: AuditLogEntry): string {
  const name = [e.targetNamespace, e.targetName].filter((p) => p !== null).join('/')
  return [e.targetKind, name].filter((p) => p !== null && p !== '').join(' ')
}

/** `datetime-local` value -> epoch ms (undefined when empty/invalid). */
function localToMs(v: string): number | undefined {
  if (v === '') return undefined
  const t = new Date(v).getTime()
  return Number.isNaN(t) ? undefined : t
}

export function AuditLogPane() {
  const audit = useStore((s) => s.audit)
  const refresh = useStore((s) => s.refreshAuditLog)
  const setVisible = useStore((s) => s.setAuditVisible)
  const setFilter = useStore((s) => s.setAuditFilter)

  useEffect(() => {
    setVisible(true)
    refresh()
    return () => {
      setVisible(false)
    }
  }, [refresh, setVisible])

  const { entries, filter } = audit
  const visible = useMemo(() => applyAuditFilter(entries, filter), [entries, filter])
  const contexts = useMemo(() => [...new Set(entries.map((e) => e.context))].sort(), [entries])
  const kinds = useMemo(() => [...new Set(entries.map((e) => e.action))].sort(), [entries])
  const filtered = !isDefaultFilter(filter)

  return (
    <div className="flex h-full min-h-0 flex-col" data-testid="audit-log">
      <p className="border-b px-3 py-2 text-xs text-muted-foreground" data-testid="audit-log-note">
        Local history of actions taken in this app, most recent 500 only. This is not
        Kubernetes&apos; own audit log.
      </p>
      <div className="flex flex-wrap items-center gap-2 border-b p-2" data-testid="audit-filters">
        <Input
          aria-label="Search target name"
          placeholder="search name…"
          value={filter.searchText}
          onChange={(e) => {
            setFilter({ searchText: e.target.value })
          }}
          className="h-[var(--touch-target)] w-40"
        />
        <select
          aria-label="Action type"
          className={SELECT_CLASS}
          value={filter.actionKinds[0] ?? ''}
          onChange={(e) => {
            const v = e.target.value as AuditActionKind | ''
            setFilter({ actionKinds: v === '' ? [] : [v] })
          }}
        >
          <option value="">All actions</option>
          {kinds.map((k) => (
            <option key={k} value={k}>
              {ACTION_LABELS[k]}
            </option>
          ))}
        </select>
        <select
          aria-label="Outcome"
          className={SELECT_CLASS}
          value={filter.outcome}
          onChange={(e) => {
            setFilter({ outcome: e.target.value as 'all' | 'success' | 'failure' })
          }}
        >
          <option value="all">Any outcome</option>
          <option value="success">Success</option>
          <option value="failure">Failed</option>
        </select>
        <select
          aria-label="Context"
          className={SELECT_CLASS}
          value={filter.context ?? ''}
          onChange={(e) => {
            setFilter({ context: e.target.value === '' ? null : e.target.value })
          }}
        >
          <option value="">All contexts</option>
          {contexts.map((c) => (
            <option key={c} value={c}>
              {c}
            </option>
          ))}
        </select>
        <Input
          type="datetime-local"
          aria-label="From"
          className="h-[var(--touch-target)] w-48"
          onChange={(e) => {
            setFilter({ timeRange: { ...filter.timeRange, from: localToMs(e.target.value) } })
          }}
        />
        <Input
          type="datetime-local"
          aria-label="To"
          className="h-[var(--touch-target)] w-48"
          onChange={(e) => {
            setFilter({ timeRange: { ...filter.timeRange, to: localToMs(e.target.value) } })
          }}
        />
        {filtered && (
          <Button
            variant="ghost"
            onClick={() => {
              setFilter({
                actionKinds: [],
                outcome: 'all',
                context: null,
                timeRange: {},
                searchText: '',
              })
            }}
          >
            Clear filters
          </Button>
        )}
      </div>
      <div className="min-h-0 flex-1 overflow-auto">
        {entries.length === 0 ? (
          <Empty
            title="No actions recorded yet"
            detail="Mutating actions you take in Fathom will appear here."
          />
        ) : visible.length === 0 ? (
          <Empty title="No matching entries" detail="Clear the filters to see the full history." />
        ) : (
          <ul className="flex flex-col" data-testid="audit-log-list">
            {visible.map((e) => (
              <li
                key={e.id}
                data-testid="audit-entry"
                className="flex flex-col gap-0.5 border-b px-3 py-2 text-sm"
              >
                <div className="flex items-center justify-between gap-2">
                  <span className="font-medium">{e.actionLabel}</span>
                  {e.outcome.type === 'Success' ? (
                    <span data-testid="audit-outcome" className="text-xs text-emerald-500">
                      Success
                    </span>
                  ) : (
                    <span data-testid="audit-outcome" className="text-xs text-destructive">
                      Failed
                    </span>
                  )}
                </div>
                <div className="truncate font-mono text-xs">{describeTarget(e)}</div>
                <div className="flex flex-wrap gap-x-3 text-xs text-muted-foreground">
                  <span className="hidden sm:inline">{e.context}</span>
                  <time dateTime={e.timestamp}>{new Date(e.timestamp).toLocaleString()}</time>
                </div>
                {e.outcome.type === 'Failure' && (
                  <div className="text-xs text-destructive">{e.outcome.data.reason}</div>
                )}
              </li>
            ))}
          </ul>
        )}
      </div>
    </div>
  )
}
