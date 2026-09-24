/** `BackendEvent::System` arm: session, notifications, integrations, persisted UI config. */
import { reduceIdentity } from '@/features/identity/state'
import { resolveProfile } from '@/layout/defaults'
import { ingestCapabilityChecked, ingestCapabilitySet, resetCapabilities } from '@/lib/capabilities'
import type { BackendEventEnvelope, NotificationLevel, SystemEvent } from '@/protocol/types'

import { commandFinished, type CommandResult } from '../actions'
import type { Draft } from '../draft'
import {
  applied as helmMutationApplied,
  applyFailed as helmMutationApplyFailed,
  previewFailed as helmMutationPreviewFailed,
  previewReady as helmMutationPreviewReady,
} from '../helmMutation'
import { assignPanes, CONTEXT_LISTS, listKey, markLoading, type Notice } from '../index'
import { RingBuffer } from '../ring-buffer'
import { applyFleetSummary } from './fleet'
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
      d.effects.push(resetCapabilities)
      return
    }

    // 033 US5: capability answers feed the `<Gated>` controls (lib/capabilities.ts). Answers
    // for a context that is no longer active are discarded there.
    case 'CapabilitiesResolved': {
      const set = event.data
      d.effects.push(() => {
        ingestCapabilitySet(set)
      })
      return
    }
    case 'CapabilityChecked': {
      const { query, result } = event.data
      d.effects.push(() => {
        ingestCapabilityChecked(query, result)
      })
      return
    }

    case 'ContextStatusUpdated': {
      // Last write per context wins (data-model.md §"Frontend") — the backend never emits both
      // a probe result and the active-context connect result for the same context
      // (research.md R3), so no reconciliation beyond a plain overwrite is needed.
      const session = d.edit('session')
      session.contextStatuses = {
        ...session.contextStatuses,
        [event.data.context]: {
          status: event.data.status,
          ...(event.data.error ? { error: event.data.error } : {}),
        },
      }
      return
    }

    case 'FleetSummaryUpdated':
      applyFleetSummary(d, event.data)
      return

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

    case 'HelmMutationPreviewReady': {
      d.put('helmMutation', helmMutationPreviewReady(event.data))
      const helmMutations = d.edit('helmMutations')
      const mutKey = `${event.data.namespace}/${event.data.release}`
      helmMutations[mutKey] = helmMutationPreviewReady(event.data)
      return
    }

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
        mcpStdioEnabled: event.data.mcp_stdio_enabled,
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

    // 006-helm-mutate: preview/apply results for the release the Inspector's Upgrade tab (or
    // rollback/uninstall counterparts) is open on. Keyed like `nodeActions` — by the target's
    // own identity, not `Correlation::Request` — since a session outlives any one command.
    case 'HelmMutationApplied': {
      d.put('helmMutation', helmMutationApplied(d.read('helmMutation')))
      const helmMutations = d.edit('helmMutations')
      const mutKey = `${event.data.namespace}/${event.data.release}`
      const prevApplied = helmMutations[mutKey]
      if (prevApplied) helmMutations[mutKey] = helmMutationApplied(prevApplied)

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

    // 013-agent-sdk-ops (US2/P1): the global assistant auth mode, replied to
    // `AgentGetAuthMode`/`AgentSetAuthMode`. Never carries the raw key (`AuthSetup.tsx`).
    // 015-mcp-agent-exposure: reply to `LoadMcpServerStatus` and broadcast after every
    // `SetMcpServerEnabled`. `running` is taken from the backend as given — it is never inferred
    // from `enabled`, so a failed start shows as enabled-but-not-running instead of healthy.
    case 'McpServerStatusChanged': {
      const mcp = d.edit('mcp')
      mcp.loaded = true
      mcp.enabled = event.data.enabled
      mcp.running = event.data.running
      mcp.connectionCommand = event.data.connection_command
      return
    }

    // 015-mcp-agent-exposure US4: the locally-launched `claude` CLI session. `sessionId` is
    // the backend's monotonic counter — chunks and endings that name an *older* session are
    // dropped, so a previous run's trailing output can never bleed into a fresh terminal.
    case 'McpDemoChatReady': {
      d.edit('mcp').demoChat = {
        status: 'running',
        sessionId: event.data.session_id,
        chunks: new RingBuffer<string>(),
        rev: 0,
      }
      return
    }

    case 'McpDemoChatOutputChunk': {
      const mcp = d.edit('mcp')
      const prev = mcp.demoChat
      if (prev.sessionId !== event.data.session_id) return
      // xterm.js wants a Latin1-per-char "binary string" so raw escape bytes survive verbatim,
      // exactly as `handlers/stream.ts` decodes `PodExecOutputChunk`.
      prev.chunks.push(atob(event.data.data))
      mcp.demoChat = { ...prev, rev: prev.rev + 1 }
      return
    }

    case 'McpDemoChatEnded': {
      const mcp = d.edit('mcp')
      if (mcp.demoChat.sessionId !== event.data.session_id) return
      mcp.demoChat = { ...mcp.demoChat, status: 'ended', reason: event.data.reason }
      return
    }

    // The backend found no `claude` on PATH. Purely backend-sourced: the UI never infers this
    // (FR-009), and it clears any session state so no stale terminal stays on screen.
    case 'McpDemoChatNotDetected': {
      d.edit('mcp').demoChat = {
        status: 'not-detected',
        instructions: event.data.instructions,
        chunks: new RingBuffer<string>(),
        rev: 0,
      }
      return
    }

    case 'AgentAuthModeUpdated': {
      d.edit('agentAuth').mode = event.data.mode
      return
    }

    // The raw key, sent only in direct reply to an explicit operator `AgentRevealKey` click
    // (FR-004, Principle VI) — never stored anywhere else, never auto-populated.
    case 'AgentKeyRevealed': {
      d.edit('agentAuth').revealedKey = event.data.key
      return
    }

    // 014-teams-organizations: local §10 Cluster Settings, reply to `LoadClusterSettings` and
    // broadcast after `SaveClusterSettings`/`AcceptOrgClusterSettings`/`DismissOrgClusterSettings`.
    // Accepting/dismissing an org default also clears `orgAvailable` for that context (the
    // backend only re-emits `OrgClusterSettingsAvailable` if a *newer* rev shows up later).
    case 'ClusterSettingsLoaded': {
      const team = d.edit('team')
      team.clusterSettings = event.data.settings
      team.clusterSettingsContext = event.data.context
      team.clusterSettingsError = undefined
      if (team.orgAvailable?.context === event.data.context) team.orgAvailable = undefined
      return
    }

    // `SaveClusterSettings`/`AcceptOrgClusterSettings` failed local validation or the config
    // write itself; prior local settings are left untouched (FR-009, Constitution Principle VI).
    case 'ClusterSettingsError': {
      const team = d.edit('team')
      team.clusterSettingsContext = event.data.context
      team.clusterSettingsError = event.data.reason
      return
    }

    case 'OidcConfigLoaded':
    case 'OidcSignInProgress':
    case 'OidcIdentityUpdated':
    case 'OidcSessionEnded':
    case 'OidcError': {
      reduceIdentity(d.edit('identity'), event)
      return
    }

    // Reply to `RefreshTeamStoreStatus`, and broadcast whenever reachability changes (FR-005).
    case 'TeamStoreStatusUpdated': {
      d.edit('team').storeStatus = event.data
      return
    }

    // Reply to `RefreshConnectionProfiles`, and broadcast after publish/adopt/remove.
    case 'ConnectionProfilesUpdated': {
      const team = d.edit('team')
      team.adopted = event.data.adopted
      team.available = event.data.available
      return
    }

    // A newer org-level Cluster Settings default is available for `context` and has not yet
    // been accepted or dismissed by this operator (FR-007).
    case 'OrgClusterSettingsAvailable': {
      d.edit('team').orgAvailable = event.data.settings
      return
    }

    // `PublishConnectionProfile`/`PublishOrgClusterSettings` was refused: a suspected secret, a
    // store-write conflict, or an unreachable store (FR-002, FR-012).
    case 'TeamPublishRejected': {
      d.edit('team').rejection = event.data.rejection
      return
    }

    case 'HelmMutationFailed': {
      d.put(
        'helmMutation',
        event.data.stage === 'preview'
          ? helmMutationPreviewFailed(event.data.message)
          : helmMutationApplyFailed(d.read('helmMutation'), event.data.message),
      )
      const helmMutations = d.edit('helmMutations')
      const mutKey = `${event.data.namespace}/${event.data.release}`
      const prev = helmMutations[mutKey]
      helmMutations[mutKey] =
        event.data.stage === 'apply' && prev
          ? helmMutationApplyFailed(prev, event.data.message)
          : helmMutationPreviewFailed(event.data.message)

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
