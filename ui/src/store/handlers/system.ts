/** `BackendEvent::System` arm: session, notifications, integrations, persisted UI config. */
import { resolveProfile } from '@/layout/defaults'
import type { BackendEventEnvelope, NotificationLevel, SystemEvent } from '@/protocol/types'

import { commandFinished, type CommandResult } from '../actions'
import type { Draft } from '../draft'
import { assignPanes, CONTEXT_LISTS, listKey, markLoading, type Notice } from '../index'
import { rowId } from './list'

const LEVEL: Record<NotificationLevel, Notice['level']> = {
  Info: 'info',
  Warn: 'warn',
  Error: 'error',
}

/** Mirrors `SECTION` in `resources/helm.tsx` — not imported directly to avoid a cycle back into
 * `@/resources` from a store handler module. */
const HELM_RELEASES_SECTION = 'helm/releases'

function helmUninstallKey(namespace: string, release: string): string {
  return `${namespace}/${release}`
}

/**
 * `BackendEvent::CommandFinished`, emitted once per command with its `Request` correlation
 * (data-model rule 5): hands the result to the action awaiting it (store/actions.ts) after the
 * batch commits, so its toast and refresh see this batch's lists.
 */
export function handleCommandFinished(
  d: Draft,
  env: BackendEventEnvelope,
  result: CommandResult,
): void {
  if (env.correlation.type !== 'Request') return
  const id = env.correlation.data
  const { ok, error } = result
  d.effects.push(() => {
    commandFinished(id, { ok, error })
  })
}

export function handleSystem(d: Draft, env: BackendEventEnvelope, event: SystemEvent): void {
  switch (event.type) {
    case 'BackendStarted':
      return

    case 'Notification': {
      const level = LEVEL[event.data.level]
      const conn = d.edit('conn')
      conn.lastNotice = { level, message: event.data.message, at: Date.now() }
      // A pending action's own error is reported by its `CommandFinished` toast; any other
      // error (e.g. a refresh's, now also `Request`-correlated) fails the loading lists.
      const fromAction =
        env.correlation.type === 'Request' && d.read('pending')[env.correlation.data] !== undefined
      if (level === 'error' && !fromAction) {
        failLoadingLists(d, event.data.message)
      }
      return
    }

    case 'ContextsUpdated': {
      const session = d.edit('session')
      session.contexts = event.data.contexts
      session.contextsLoaded = true
      if (session.activeContext === undefined && event.data.current !== null) {
        session.activeContext = event.data.current
      }
      return
    }

    case 'ContextChanged': {
      const current = event.data.current ?? undefined
      if (d.read('session').activeContext === current) return
      // Backend-initiated switch: the previous context's namespaces no longer apply.
      const session = d.edit('session')
      session.activeContext = current
      session.namespaces = []
      session.activeNamespace = null
      session.namespaceFilter = []
      d.put('lists', markLoading(d.read('lists'), current, CONTEXT_LISTS))
      return
    }

    case 'NamespaceFilterChanged': {
      const session = d.edit('session')
      const namespaces = event.data.namespaces
      session.namespaceFilter = namespaces
      session.activeNamespace = namespaces.length === 1 ? (namespaces[0] ?? null) : null
      return
    }

    case 'PrometheusConnectionTest':
      d.edit('integrations').prometheusTest = event.data
      return
    case 'GrafanaConnectionTest':
      d.edit('integrations').grafanaTest = event.data
      return
    case 'McpServerStatus':
      d.edit('integrations').mcp = event.data
      return
    case 'PortAvailability': {
      const integrations = d.edit('integrations')
      integrations.portAvailability = {
        ...integrations.portAvailability,
        [event.data.port]: event.data.available,
      }
      return
    }
    case 'MetricsStackScanned':
      d.edit('metrics').stackScan = event.data
      return
    case 'DiskUsageUpdated':
      d.edit('metrics').diskUsage = event.data
      return
    case 'FluxOverviewUpdated':
      d.put('flux', { overview: event.data })
      return
    case 'HubbleTrafficUpdated':
      d.put('hubble', { summary: event.data })
      return

    case 'CrdCatalogUpdated': {
      const crds = d.edit('crds')
      crds.catalog = event.data.entries
      crds.catalogLoaded = true
      // C1: a fresh catalog that no longer lists the currently-selected kind — mark its last
      // instance list 'unavailable' rather than leaving it stale/silently-empty.
      const sel = crds.selected
      if (sel) {
        const stillPresent = event.data.entries.some(
          (e) => e.group === sel.group && e.kind === sel.kind,
        )
        if (!stillPresent) {
          const key = `${sel.group}/${sel.kind}`
          const prev = crds.instances[key]
          crds.instances = {
            ...crds.instances,
            [key]: {
              status: 'unavailable',
              namespace: prev?.namespace ?? null,
              items: [],
              updatedAt: Date.now(),
            },
          }
        }
      }
      return
    }

    case 'CrdInstancesUpdated': {
      const crds = d.edit('crds')
      const key = `${event.data.group}/${event.data.kind}`
      crds.instances = {
        ...crds.instances,
        [key]: {
          status: 'ready',
          namespace: event.data.namespace,
          items: event.data.instances,
          updatedAt: Date.now(),
        },
      }
      return
    }

    case 'UiConfigLoaded': {
      d.put('prefs', {
        loaded: true,
        expandedGroups: event.data.expanded_groups,
        mcpAutoStart: event.data.mcp_auto_start,
      })
      // `null` = nothing saved (or unreadable): keep the defaults (SC-009).
      const ui = event.data.ui
      if (ui === null) return
      d.put('ui', ui)
      const layout = d.read('layout')
      const profile = resolveProfile(
        layout.widthClass,
        ui.layouts[layout.widthClass],
        layout.ultraPlus,
      )
      const { slotPanes, dockPanes, panes } = assignPanes(profile, d.read('panes'))
      d.put('panes', panes)
      d.put('layout', { ...layout, profile, slotPanes, dockPanes })
      return
    }

    case 'HelmMutationApplied': {
      if (event.data.action !== 'uninstall') return
      const { namespace, release } = event.data
      const key = helmUninstallKey(namespace, release)
      d.edit('helmUninstalls')[key] = { status: 'complete' }

      // T035: drop the release from the list state immediately (no need to wait for a refresh)
      // and close the Inspector if it was open on the now-gone release.
      const ctx = d.read('session').activeContext
      const listK = listKey(ctx, HELM_RELEASES_SECTION, 'Namespaced', null)
      const list = d.read('lists')[listK]
      if (list) {
        const removedId = `${namespace}/${release}`
        d.edit('lists')[listK] = {
          ...list,
          rows: list.rows.filter((r) => rowId(r) !== removedId),
          updatedAt: Date.now(),
        }
      }

      const panes = d.read('panes')
      let panesChanged = false
      const nextPanes = { ...panes }
      for (const [id, pane] of Object.entries(panes)) {
        if (
          pane.kind === 'inspector' &&
          pane.target?.section === HELM_RELEASES_SECTION &&
          pane.target.namespace === namespace &&
          pane.target.name === release
        ) {
          nextPanes[id] = { ...pane, target: undefined, source: undefined }
          panesChanged = true
        }
      }
      if (panesChanged) d.put('panes', nextPanes)
      return
    }

    case 'HelmMutationFailed': {
      if (event.data.action !== 'uninstall') return
      const key = helmUninstallKey(event.data.namespace, event.data.release)
      d.edit('helmUninstalls')[key] = { status: 'failed', error: event.data.message }
      return
    }
  }
}

/**
 * Protocol v1 reports a failed refresh only as an uncorrelated error `Notification` (the
 * backend's `notify_error`). Attribute it to the active context's lists that are still loading,
 * so their panes show the error verbatim instead of spinning forever.
 * TODO(protocol): route list failures by `Correlation::Request` or `Route` instead.
 */
function failLoadingLists(d: Draft, message: string): void {
  const ctx = d.read('session').activeContext
  if (ctx === undefined) return
  const prefix = `${ctx}|`
  const lists = d.read('lists')
  const failing = Object.keys(lists).filter(
    (k) => k.startsWith(prefix) && lists[k]?.status === 'loading',
  )
  // Lists the first `Initialize` never delivered have no entry yet (the pane shows Loading).
  const missing = CONTEXT_LISTS.map((sec) => listKey(ctx, sec, 'Namespaced', null)).filter(
    (k) => !(k in lists),
  )
  if (failing.length === 0 && missing.length === 0) return
  const edit = d.edit('lists')
  for (const k of failing) {
    const prev = lists[k]
    if (prev) edit[k] = { ...prev, status: 'error', error: message }
  }
  for (const k of missing) edit[k] = { rows: [], status: 'error', error: message, updatedAt: 0 }
}
