/**
 * Cluster Settings panel (014-teams-organizations, expanded Story 2): the local, per-operator
 * §10 field set (metrics source, Prometheus path, HTTPS toggle, path prefix, hidden metrics),
 * plus org-level sync affordances — publish this context's settings as the team default, and
 * accept/dismiss a newer org default someone else published (FR-006..FR-012).
 *
 * Settings are per-context: this panel re-loads via `LoadClusterSettings` whenever the active
 * context changes, and only shows the loaded settings once they match that context
 * (`team.clusterSettingsContext === activeContext`) — otherwise a stale previous context's
 * settings would flash before the new load lands.
 *
 * All new testids are prefixed `teams-`.
 *
 * 043 US3: this page holds only per-context settings (metrics, org default, node shell, cloud
 * auth exec policy, the in-cluster daemon). Appearance, the AI assistant and available agents
 * moved to Settings › App settings, and local kind clusters to Settings › Local clusters.
 */
import { useEffect, useState } from 'react'

import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import type { ClusterSettings, MetricsSource } from '@/protocol/types'
import { useStore } from '@/store'
import { waitForCommandResult } from '@/store/actions'

import { useFirstRun } from '@/lib/use-first-run'

import { FirstRunState } from '../shell/FirstRunState'
import { Disconnected, Empty, Loading } from '../shell/PaneStates'

import { DaemonSwitch } from '@/features/daemon/DaemonSwitch'
import { SECTION } from '@/protocol/commands'
import { CloudAuthSection } from './CloudAuthSection'
import { NodeShellSettingsSection } from './NodeShellSettingsSection'

function cloneSettings(s: ClusterSettings): ClusterSettings {
  return { ...s, hiddenMetrics: [...s.hiddenMetrics], fieldOrigin: { ...s.fieldOrigin } }
}

export function ClusterSettingsPanel() {
  const request = useStore((s) => s.request)
  const hosted = useStore((s) => s.hosted.mode === 'hosted')
  const send = useStore((s) => s.send)
  const activeContext = useStore((s) => s.session.activeContext)
  const disconnected = useStore((s) => s.conn.status === 'backend-gone')
  const firstRun = useFirstRun()
  const loaded = useStore((s) => s.team.clusterSettings)
  const loadedContext = useStore((s) => s.team.clusterSettingsContext)
  const loadError = useStore((s) => s.team.clusterSettingsError)
  const orgAvailable = useStore((s) => s.team.orgAvailable)
  const rejection = useStore((s) => s.team.rejection)
  const navigate = useStore((s) => s.navigate)

  const [draft, setDraft] = useState<ClusterSettings | undefined>(undefined)
  const [hiddenMetricInput, setHiddenMetricInput] = useState('')
  const [saving, setSaving] = useState(false)
  const [publishing, setPublishing] = useState(false)
  const [savedAt, setSavedAt] = useState<number | undefined>(undefined)
  const [formError, setFormError] = useState<string | undefined>(undefined)

  // Re-load whenever the active context changes (matches PortForwardsPanel's
  // "send only on mount/dep change" contract).
  useEffect(() => {
    if (activeContext === undefined) return
    send({ type: 'LoadClusterSettings', data: { context: activeContext } })
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [activeContext])

  // Prefill the draft form once from the freshly loaded settings for the active context, then
  // never again until the context changes again — same "adjust state during render, guarded by
  // comparison" pattern as `AuthSetup.tsx`, so a save-in-progress edit is never clobbered by a
  // subsequent broadcast of the same settings.
  const [prefilledFor, setPrefilledFor] = useState<string | undefined>(undefined)
  if (loaded !== undefined && loadedContext === activeContext && prefilledFor !== loadedContext) {
    setPrefilledFor(loadedContext)
    setDraft(cloneSettings(loaded))
  }

  if (disconnected) return <Disconnected />
  if (activeContext === undefined || firstRun?.kind === 'kubeconfig') {
    // First run: there are no per-context settings yet. Say so, and point at Local clusters
    // (030), where a kind cluster can be created without any kubeconfig.
    return (
      <div data-testid="cluster-settings-first-run" className="flex flex-col gap-4 p-3">
        <p data-testid="cluster-settings-needs-context" className="text-sm">
          Cluster settings belong to a context. Choose or connect one to change them.
        </p>
        {firstRun ? (
          <FirstRunState state={firstRun} inSettings />
        ) : (
          <Empty title="No active context" detail="Connect to a cluster to configure settings." />
        )}
        {!hosted && (
          <div>
            <Button
              type="button"
              variant="outline"
              data-testid="cluster-settings-open-local-clusters"
              onClick={() => navigate(SECTION.localClusters)}
            >
              Create a local cluster
            </Button>
          </div>
        )}
      </div>
    )
  }
  if (loadError !== undefined && loadedContext === activeContext && draft === undefined) {
    return <Empty title="Could not load Cluster Settings" detail={loadError} />
  }
  if (draft === undefined || prefilledFor !== activeContext) {
    return <Loading label="Loading Cluster Settings" />
  }

  const save = () => {
    if (saving) return
    setSaving(true)
    setFormError(undefined)
    setSavedAt(undefined)
    void (async () => {
      try {
        const requestId = await request({
          type: 'SaveClusterSettings',
          data: { context: activeContext, settings: draft },
        })
        const result = await waitForCommandResult(requestId)
        if (result === 'timeout') setFormError('The backend did not respond in time.')
        else if (!result.ok) setFormError(result.error ?? 'Saving Cluster Settings failed.')
        else setSavedAt(Date.now())
      } catch (err) {
        setFormError(err instanceof Error ? err.message : String(err))
      } finally {
        setSaving(false)
      }
    })()
  }

  const publishOrgDefault = () => {
    if (publishing) return
    setPublishing(true)
    setFormError(undefined)
    void (async () => {
      try {
        const requestId = await request({
          type: 'PublishOrgClusterSettings',
          data: { context: activeContext },
        })
        const result = await waitForCommandResult(requestId)
        if (result === 'timeout') setFormError('The backend did not respond in time.')
        else if (!result.ok) setFormError(result.error ?? 'Publishing the org default failed.')
      } catch (err) {
        setFormError(err instanceof Error ? err.message : String(err))
      } finally {
        setPublishing(false)
      }
    })()
  }

  const acceptOrgDefault = () => {
    send({ type: 'AcceptOrgClusterSettings', data: { context: activeContext } })
  }
  const dismissOrgDefault = () => {
    send({ type: 'DismissOrgClusterSettings', data: { context: activeContext } })
  }

  const addHiddenMetric = () => {
    const value = hiddenMetricInput.trim()
    if (value === '' || draft.hiddenMetrics.includes(value)) return
    setDraft({ ...draft, hiddenMetrics: [...draft.hiddenMetrics, value] })
    setHiddenMetricInput('')
  }
  const removeHiddenMetric = (metric: string) => {
    setDraft({ ...draft, hiddenMetrics: draft.hiddenMetrics.filter((m) => m !== metric) })
  }

  return (
    <div
      data-testid="teams-cluster-settings-panel"
      className="flex h-full min-h-0 flex-col gap-4 overflow-auto p-4"
    >
      {orgAvailable !== undefined && (
        <div
          data-testid="teams-org-default-banner"
          role="status"
          className="flex flex-col gap-2 border border-accent-neon/60 bg-accent-neon/10 p-3 text-xs"
        >
          <p>
            <strong>{orgAvailable.publishedBy}</strong> published an updated org-level default for{' '}
            <strong>{activeContext}</strong> (rev {orgAvailable.rev}).
          </p>
          <div className="flex gap-2">
            <Button
              type="button"
              size="sm"
              data-testid="teams-accept-org-default"
              onClick={acceptOrgDefault}
            >
              Accept
            </Button>
            <Button
              type="button"
              size="sm"
              variant="outline"
              data-testid="teams-dismiss-org-default"
              onClick={dismissOrgDefault}
            >
              Dismiss
            </Button>
          </div>
        </div>
      )}

      {rejection !== undefined && (
        <p
          role="alert"
          data-testid="teams-publish-rejection"
          className="border border-status-err/50 px-2 py-1 text-xs text-status-err"
        >
          {rejectionMessage(rejection)}
        </p>
      )}

      <fieldset className="flex flex-col gap-3">
        <legend className="text-xs font-semibold tracking-wider uppercase">Metrics</legend>

        <label className="flex flex-col gap-1 text-xs">
          Metrics source
          <select
            data-testid="teams-metrics-source"
            value={draft.metricsSource}
            onChange={(e) => setDraft({ ...draft, metricsSource: e.target.value as MetricsSource })}
            className="h-[var(--touch-target)] border border-input bg-background px-2 text-sm"
          >
            <option value="none">None</option>
            <option value="prometheus">Prometheus</option>
          </select>
        </label>

        <label htmlFor="teams-prometheus-path" className="flex flex-col gap-1 text-xs">
          Prometheus path
          <Input
            id="teams-prometheus-path"
            data-testid="teams-prometheus-path"
            value={draft.prometheusPath}
            onChange={(e) => setDraft({ ...draft, prometheusPath: e.target.value })}
          />
        </label>

        <label className="flex items-center gap-2 text-xs">
          <input
            type="checkbox"
            data-testid="teams-https-toggle"
            checked={draft.https}
            onChange={(e) => setDraft({ ...draft, https: e.target.checked })}
          />
          Use HTTPS
        </label>

        <label htmlFor="teams-path-prefix" className="flex flex-col gap-1 text-xs">
          Path prefix
          <Input
            id="teams-path-prefix"
            data-testid="teams-path-prefix"
            value={draft.pathPrefix}
            onChange={(e) => setDraft({ ...draft, pathPrefix: e.target.value })}
          />
        </label>

        <div className="flex flex-col gap-1 text-xs">
          <span>Hidden metrics</span>
          <div className="flex gap-2">
            <Input
              data-testid="teams-hidden-metric-input"
              value={hiddenMetricInput}
              onChange={(e) => setHiddenMetricInput(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === 'Enter') {
                  e.preventDefault()
                  addHiddenMetric()
                }
              }}
              placeholder="metric name"
            />
            <Button type="button" size="sm" variant="outline" onClick={addHiddenMetric}>
              Add
            </Button>
          </div>
          <ul data-testid="teams-hidden-metrics-list" className="flex flex-wrap gap-1">
            {draft.hiddenMetrics.map((metric) => (
              <li
                key={metric}
                data-testid="teams-hidden-metric-row"
                className="flex items-center gap-1 border border-border px-2 py-0.5"
              >
                {metric}
                <button
                  type="button"
                  aria-label={`Remove ${metric}`}
                  data-testid="teams-remove-hidden-metric"
                  onClick={() => removeHiddenMetric(metric)}
                >
                  ✖
                </button>
              </li>
            ))}
          </ul>
        </div>
      </fieldset>

      {!hosted && (
        <>
          <NodeShellSettingsSection />
          <CloudAuthSection />
          {/* 043 US4: the same daemon switch and explanation as the header Daemon chip. */}
          <fieldset className="flex flex-col gap-2" data-testid="cluster-settings-daemon">
            <legend className="text-xs font-semibold tracking-wider uppercase">
              In-cluster daemon
            </legend>
            <DaemonSwitch testId="cluster-settings-daemon-toggle" />
          </fieldset>
        </>
      )}

      {formError !== undefined && (
        <p
          role="alert"
          data-testid="teams-cluster-settings-error"
          className="text-xs text-status-err"
        >
          {formError}
        </p>
      )}
      {savedAt !== undefined && (
        <p
          role="status"
          data-testid="teams-cluster-settings-saved"
          className="text-xs text-status-ok"
        >
          Saved.
        </p>
      )}

      <div className="flex gap-2">
        <Button
          type="button"
          data-testid="teams-save-cluster-settings"
          onClick={save}
          disabled={saving}
        >
          {saving ? 'Saving…' : 'Save'}
        </Button>
        <Button
          type="button"
          variant="outline"
          data-testid="teams-publish-org-default"
          onClick={publishOrgDefault}
          disabled={publishing}
        >
          {publishing ? 'Publishing…' : 'Publish as org default'}
        </Button>
      </div>
    </div>
  )
}

function rejectionMessage(rejection: {
  kind: string
  field?: string
  reason?: string
  expected_rev?: number
  current_rev?: number
}): string {
  switch (rejection.kind) {
    case 'suspectedSecret':
      return `Publish refused: field "${rejection.field}" ${rejection.reason}`
    case 'conflict':
      return `Publish refused: someone else published rev ${rejection.current_rev} first (you had rev ${rejection.expected_rev}). Reload and retry.`
    case 'storeUnreachable':
      return `Publish refused: the shared team store is unreachable (${rejection.reason}).`
    case 'roleForbidden':
      return `Not permitted: ${rejection.reason}`
    default:
      return 'Publish refused.'
  }
}
