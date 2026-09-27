/** `BackendEvent::System` arm: session, notifications, integrations, persisted UI config. */
import { reduceIdentity } from '@/features/identity/state'
import { reduceTeams } from '@/features/teams/state'
import { reduceRoles } from '@/features/roles/state'
import { reduceManifestExport } from '@/features/manifest-export/state'
import { resolveProfile } from '@/layout/defaults'
import { connectionProblemSummary } from '@/lib/first-run'
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
import {
  assignPanes,
  CONTEXT_LISTS,
  listKey,
  markLoading,
  type Notice,
  serviceForwardKey,
  useStore,
  withoutKey,
} from '../index'
import { RingBuffer } from '../ring-buffer'
import {
  applyDaemonBatch,
  applyDaemonMetrics,
  applyDaemonStatus,
  applyDaemonTopology,
} from './daemon'
import { applyFederationRollup, applyFederationState } from './federation'
import { applyFleetSummary } from './fleet'
import { clearDenied, deniedKeyOf, rowId } from './list'

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
  // 031: a section whose refresh now succeeds is no longer denied.
  if (ok) clearDenied(d, env.route.context, env.route.section)
  // 026: a finished command may have appended an audit entry; refresh the viewer if it is open
  // (never for the audit refresh's own completion, which would loop).
  const audit = d.read('audit')
  const refreshAudit = audit.visible && audit.lastLoadedRequest !== id
  d.effects.push(() => {
    commandFinished(id, { ok, error })
    if (refreshAudit) useStore.getState().refreshAuditLog()
  })
}

export function handleSystem(d: Draft, env: BackendEventEnvelope, event: SystemEvent): void {
  switch (event.type) {
    // A transient reconnect re-delivers this: nothing to do, and never resets hosted state.
    case 'BackendStarted':
      return

    case 'SessionEnded':
      d.put('hosted', { ...d.read('hosted'), sessionEnded: event.data.reason })
      return
    case 'LimitReached':
      d.put('hosted', {
        ...d.read('hosted'),
        limitReached: { kind: event.data.kind, max: event.data.max },
      })
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
        // A refresh failure carries its section's route (the backend scopes errors raised by a
        // section command); a global route is a genuinely uncorrelated error.
        const section = env.route.section
        failLoadingLists(
          d,
          event.data.message,
          env.correlation.type === 'Request' && section !== 'system' ? section : undefined,
        )
      }
      return
    }

    // 031: the identity may not read this section (an RBAC 403). Its pane shows "Not
    // permitted" instead of an error toast; an action's own denial is its toast's business.
    case 'AccessDenied': {
      const fromAction =
        env.correlation.type === 'Request' && d.read('pending')[env.correlation.data] !== undefined
      const section = env.route.section
      if (fromAction || section === 'system') return
      d.edit('denied')[deniedKeyOf(d, env.route.context, section)] = event.data.message
      failLoadingLists(d, event.data.message, section, true)
      return
    }

    case 'NamespaceAccess':
      d.edit('session').namespacesRestricted = event.data.restricted
      return

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
      // 018: cached permission answers belong to the previous context's identity.
      d.put('rbacAccess', {})
      d.put('denied', {})
      session.namespacesRestricted = undefined
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
          ...(event.data.plugin_error ? { pluginError: event.data.plugin_error } : {}),
        },
      }
      return
    }

    // First-run UX: the kubeconfig's state (null = usable), re-sent with every context reload.
    case 'KubeconfigStatus': {
      const session = d.edit('session')
      if (event.data.issue === null) delete session.kubeconfigIssue
      else session.kubeconfigIssue = event.data.issue
      return
    }

    // First-run UX: why a context cannot be used (null = it connected). The panes show the
    // explanation instead of their lists; the lists still loading fail with the one-line summary
    // so no pane is left on a skeleton that never resolves.
    case 'ActiveConnectionProblem': {
      const { context, problem } = event.data
      const session = d.edit('session')
      session.connectionProblems =
        problem === null
          ? withoutKey(session.connectionProblems, context)
          : { ...session.connectionProblems, [context]: problem }
      if (problem !== null && session.activeContext === context) {
        const summary = connectionProblemSummary(context, problem)
        failLoadingLists(d, summary)
        // Replace a stale "Connecting to cluster..." in the status bar (no toast: the panes
        // already explain the problem).
        d.edit('conn').lastNotice = { level: 'warn', message: summary, at: Date.now() }
      }
      return
    }

    case 'CloudAuthDetected': {
      const cloudAuth: Record<string, (typeof event.data.infos)[number]> = {}
      for (const info of event.data.infos) cloudAuth[info.context] = info
      d.edit('session').cloudAuth = cloudAuth
      return
    }

    case 'FleetSummaryUpdated':
      applyFleetSummary(d, event.data)
      return

    case 'DaemonStatusUpdated':
      applyDaemonStatus(d, event.data)
      return

    case 'DaemonEventsBatch':
      applyDaemonBatch(d, event.data)
      return

    case 'DaemonTopologyUpdated':
      applyDaemonTopology(d, event.data)
      return

    case 'DaemonMetricsUpdated':
      applyDaemonMetrics(d, event.data)
      return

    case 'FederationStateUpdated':
      applyFederationState(d, event.data)
      return

    case 'FederationRollupUpdated':
      applyFederationRollup(d, event.data)
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
    case 'ObservabilitySettings': {
      // 039: settings are per context; a late reply for a previous context is dropped.
      const active = d.read('session').activeContext ?? 'default'
      if (event.data.context !== active) return
      const integrations = d.edit('integrations')
      integrations.settings = {
        ...integrations.settings,
        [event.data.context]: {
          prometheus: event.data.prometheus,
          grafana: event.data.grafana,
        },
      }
      return
    }
    case 'ServicePortForwardReady': {
      const { namespace, service, port, local_port, tunnel_id } = event.data
      const integrations = d.edit('integrations')
      integrations.serviceForwards = {
        ...integrations.serviceForwards,
        [serviceForwardKey(namespace, service, port)]: {
          localPort: local_port,
          tunnelId: tunnel_id,
        },
      }
      // Tag the tunnel in the Port forwards list with the Service it came from (009 origins).
      d.put('portForwardOrigins', { ...d.read('portForwardOrigins'), [tunnel_id]: { service } })
      return
    }
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
    case 'VeleroOverviewUpdated':
      d.put('velero', { overview: event.data })
      return
    case 'FluxOverviewUpdated':
      d.put('flux', { overview: event.data })
      return
    case 'ClusterVersionUpdated':
      d.put('clusterVersion', event.data)
      return
    case 'GatekeeperOverviewUpdated':
      d.put('gatekeeper', { overview: event.data })
      return
    case 'CertManagerOverviewUpdated':
      d.edit('certManager').overview = event.data
      return
    case 'CertificateChainUpdated':
      d.edit('certManager').chain = event.data
      return
    case 'TrivyOverviewUpdated':
      d.put('trivy', { overview: event.data })
      return
    case 'KnativeOverviewUpdated':
      d.put('knative', { overview: event.data })
      return
    case 'KedaOverviewUpdated':
      d.edit('keda').overview = event.data
      return
    case 'KedaResourceDetailsUpdated':
      d.edit('keda').selected = { state: 'ready', details: event.data }
      return
    case 'KedaResourceNotFound':
      d.edit('keda').selected = { state: 'gone', ...event.data }
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

    // 026: reply to `RefreshAuditLog`; the backend sends oldest first, the viewer shows newest first.
    case 'AuditLogLoaded': {
      const audit = d.edit('audit')
      audit.entries = [...event.data.entries].reverse()
      audit.loaded = true
      if (env.correlation.type === 'Request') audit.lastLoadedRequest = env.correlation.data
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
      d.put('prefs', { ...d.read('prefs'), uiEpoch: (d.read('prefs').uiEpoch ?? 0) + 1 })
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

    case 'AiProviderStatus': {
      const ai = d.edit('ai')
      ai.loaded = true
      ai.statuses = event.data.statuses
      ai.active = event.data.active
      ai.offline = event.data.offline
      ai.ollamaModel = event.data.ollama_model
      return
    }
    case 'AiLoginFlowUpdate': {
      d.edit('ai').loginFlow = event.data.flow
      return
    }
    case 'AiOllamaState': {
      const ai = d.edit('ai')
      ai.ollama = event.data.state
      ai.ollamaModel = event.data.selected_model
      return
    }
    case 'AiCredentialMigration': {
      d.edit('ai').migration = event.data.outcome
      return
    }
    case 'AiError': {
      d.edit('ai').lastError = {
        code: event.data.code,
        message: event.data.message_verbatim,
        hint: event.data.hint,
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
    // 037-node-shell. A status for an older session generation than the one on screen is stale
    // (superseded or already removed) and never overwrites it.
    case 'NodeShellStatus': {
      const ns = d.edit('nodeShell')
      const cur = ns.session
      if (cur !== undefined && event.data.session_id < cur.sessionId) return
      // A failure's verbatim detail survives the Terminating/Removed events that follow it.
      const carried =
        cur?.sessionId === event.data.session_id &&
        (cur.phase === 'Failed' || cur.phase === 'Terminating') &&
        event.data.detail === null
          ? cur.detail
          : undefined
      ns.session = {
        sessionId: event.data.session_id,
        phase: event.data.phase,
        node: event.data.node,
        namespace: event.data.namespace,
        pod: event.data.pod,
        detail: event.data.detail ?? carried,
        leftover: event.data.leftover,
      }
      return
    }

    case 'NodeShellAccessChecked':
      d.edit('nodeShell').access = {
        namespace: event.data.namespace,
        checks: event.data.checks,
      }
      return

    case 'NodeShellOrphans':
      d.edit('nodeShell').orphans = event.data.pods
      return

    case 'NodeShellSettingsLoaded': {
      const ns = d.edit('nodeShell')
      ns.settings = event.data.settings
      ns.settingsContext = event.data.context
      return
    }

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

    case 'IdpAdminKeyStatus':
    case 'TeamLinksUpdated': {
      reduceTeams(d.edit('teams'), event)
      return
    }
    case 'IdpDirectoryUpdated': {
      // Feeds both the Teams directory and the role-assignment subject picker.
      reduceTeams(d.edit('teams'), event)
      reduceRoles(d.edit('roles'), event)
      return
    }
    case 'RoleAssignmentPreview':
    case 'OrphanedSubjectsUpdated': {
      reduceRoles(d.edit('roles'), event)
      return
    }
    case 'ManifestsFetched':
    case 'KubeconfigExportPreviewed':
    case 'KubeconfigExportFinished': {
      reduceManifestExport(d.edit('manifestExport'), event)
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
 * Fails loading lists of the active context with a refresh's error, so their panes show it
 * verbatim instead of spinning forever. With `section` (the failed request's route) only that
 * section's lists fail; without it (a genuinely uncorrelated error) every loading list does.
 */
function failLoadingLists(d: Draft, message: string, section?: string, forbidden = false): void {
  const ctx = d.read('session').activeContext
  if (ctx === undefined) return
  const prefix = section === undefined ? `${ctx}|` : `${ctx}|${section}|`
  const lists = d.read('lists')
  const failing = Object.keys(lists).filter(
    (k) => k.startsWith(prefix) && lists[k]?.status === 'loading',
  )
  // Lists the first `Initialize` never delivered have no entry yet (the pane shows Loading).
  const missing = CONTEXT_LISTS.filter((sec) => section === undefined || sec === section)
    .map((sec) => listKey(ctx, sec, 'Namespaced', null))
    .filter((k) => !(k in lists))
  if (failing.length === 0 && missing.length === 0) return
  const edit = d.edit('lists')
  for (const k of failing) {
    const prev = lists[k]
    if (prev) edit[k] = { ...prev, status: 'error', error: message, forbidden }
  }
  for (const k of missing) {
    edit[k] = { rows: [], status: 'error', error: message, forbidden, updatedAt: 0 }
  }
}
