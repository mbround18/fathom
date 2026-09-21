/** `BackendEvent::System` arm: session, notifications, integrations, persisted UI config. */
import { toast } from 'sonner'

import { resolveProfile } from '@/layout/defaults'
import type { BackendEventEnvelope, NotificationLevel, SystemEvent } from '@/protocol/types'

import type { Draft } from '../draft'
import { assignPanes, type Notice } from '../index'

const LEVEL: Record<NotificationLevel, Notice['level']> = {
  Info: 'info',
  Warn: 'warn',
  Error: 'error',
}

export function handleSystem(d: Draft, env: BackendEventEnvelope, event: SystemEvent): void {
  switch (event.type) {
    case 'BackendStarted':
      return

    case 'Notification': {
      const level = LEVEL[event.data.level]
      const conn = d.edit('conn')
      conn.lastNotice = { level, message: event.data.message, at: Date.now() }
      if (env.correlation.type === 'Request') {
        resolvePending(d, env.correlation.data, level !== 'error', event.data.message)
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
  }
}

/** Data-model rule 5: an action result resolves its pending entry with a toast. */
function resolvePending(d: Draft, requestId: number, ok: boolean, message: string): void {
  const action = d.read('pending')[requestId]
  if (!action) return
  d.put('pending', without(d.read('pending'), String(requestId)))
  if (ok && action.paneId !== undefined) {
    d.put('selection', without(d.read('selection'), action.paneId))
  }
  d.effects.push(() => {
    if (ok) toast.success(action.label, { description: message })
    else toast.error(action.label, { description: message })
  })
}

function without<V>(record: Record<string, V>, key: string): Record<string, V> {
  return Object.fromEntries(Object.entries(record).filter(([k]) => k !== key))
}
