/**
 * Helm release "Upgrade" tab (006-helm-mutate US2, T027/T028): edit values, preview the diff
 * via `PreviewHelmUpgrade` (backend `helm upgrade --dry-run`, no mutation), then confirm-gate
 * the real `ApplyHelmUpgrade`. Mirrors `NodeActionsPanel`'s `ConfirmDialog` gate (constitution
 * I: every mutating action is explicit, diff-previewed, and confirmation-gated) and reuses
 * `YamlDiffView`'s generalized two-manifest mode (T017), same as the rollback flow in
 * `resources/helm.tsx`.
 */
import { useId, useState } from 'react'

import { ConfirmDialog } from '@/components/ConfirmDialog'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { GitOpsWarningBanner } from '@/components/panels/GitOpsWarningBanner'
import { YamlDiffView } from '@/components/panels/YamlDiffView'
import type { HelmReleaseDetails } from '@/protocol/types'
import { useStore } from '@/store'
import { reset as resetSession, startApply, startPreview } from '@/store/helmMutation'

export function HelmUpgradePanel({
  namespace,
  release,
  detail,
}: {
  namespace: string
  release: string
  detail: HelmReleaseDetails | undefined
}) {
  const key = `${namespace}/${release}`
  const chartVersionInputId = useId()
  const request = useStore((s) => s.request)
  const setHelmMutation = useStore((s) => s.setHelmMutation)
  const session = useStore((s) => s.helmMutations[key])

  const [valuesYaml, setValuesYaml] = useState('')
  const [chartVersion, setChartVersion] = useState('')
  const [seededFor, setSeededFor] = useState<string | undefined>(undefined)
  const [confirming, setConfirming] = useState(false)

  // Seed the textarea from the release's current user-supplied values exactly once per release,
  // when the detail first loads — adjusting state during render (not in an Effect) so the
  // operator's in-progress edits are never clobbered by a background refresh, and no extra
  // render pass is needed (react.dev "You Might Not Need an Effect").
  if (detail && seededFor !== key) {
    setSeededFor(key)
    setValuesYaml(detail.values_user_supplied)
  }

  const applying = session !== undefined && session.status === 'applying'
  const busy = (session !== undefined && session.status === 'previewing') || applying

  const runPreview = () => {
    setHelmMutation(key, startPreview())
    void request({
      type: 'PreviewHelmUpgrade',
      data: {
        namespace,
        release,
        values_yaml: valuesYaml,
        chart_version: chartVersion.trim() || null,
      },
    })
  }

  const onConfirmApply = () => {
    setConfirming(false)
    if (!session?.preview) return
    setHelmMutation(key, startApply(session))
    void request({
      type: 'ApplyHelmUpgrade',
      data: {
        namespace,
        release,
        values_yaml: valuesYaml,
        chart_version: chartVersion.trim() || null,
        base_revision: session.preview.base_revision,
      },
    })
  }

  const startOver = () => {
    setHelmMutation(key, resetSession())
  }

  const preview = session?.preview

  return (
    <div className="flex h-full min-h-0 flex-col gap-3 overflow-auto p-3">
      <section className="flex flex-col gap-2">
        <h3 className="font-display text-xs font-semibold tracking-wider text-accent-neon uppercase">
          Values
        </h3>
        <textarea
          data-testid="helm-upgrade-values"
          value={valuesYaml}
          onChange={(e) => {
            setValuesYaml(e.target.value)
          }}
          disabled={busy}
          spellCheck={false}
          rows={12}
          className="min-h-0 w-full resize-y border border-border bg-background p-2 font-mono text-xs leading-relaxed"
        />
        <label htmlFor={chartVersionInputId} className="flex items-center gap-2 text-xs">
          Chart version (optional — only the currently-installed chart can be re-rendered locally)
          <Input
            id={chartVersionInputId}
            data-testid="helm-upgrade-chart-version"
            value={chartVersion}
            onChange={(e) => {
              setChartVersion(e.target.value)
            }}
            disabled={busy}
            className="h-7 w-32 font-mono"
          />
        </label>
        <div className="flex gap-2">
          <Button
            type="button"
            variant="outline"
            size="sm"
            disabled={busy || valuesYaml.trim() === ''}
            data-testid="helm-upgrade-preview"
            onClick={runPreview}
          >
            {session?.status === 'previewing' ? 'Previewing…' : 'Preview upgrade'}
          </Button>
          {session && session.status !== 'idle' && (
            <Button type="button" variant="ghost" size="sm" onClick={startOver} disabled={busy}>
              Start over
            </Button>
          )}
        </div>
      </section>

      {session?.status === 'preview_failed' && (
        <p data-testid="helm-upgrade-error" className="text-xs text-status-err">
          {session.lastError}
        </p>
      )}
      {session?.status === 'apply_failed' && (
        <p data-testid="helm-upgrade-error" className="text-xs text-status-err">
          {session.lastError}
        </p>
      )}
      {session?.status === 'applied' && (
        <p data-testid="helm-upgrade-applied" className="text-xs text-status-ok">
          Upgrade applied.
        </p>
      )}

      {preview && (
        <section className="flex min-h-0 flex-1 flex-col gap-2 border-t border-border pt-3">
          <h3 className="font-display text-xs font-semibold tracking-wider text-accent-neon uppercase">
            Diff preview
          </h3>
          {preview.gitops_warning && (
            <GitOpsWarningBanner
              origin={preview.gitops_warning}
              testId="helm-upgrade-gitops-warning"
              consequence="A manual upgrade may be reverted or conflict with the next GitOps reconciliation."
            />
          )}
          <YamlDiffView
            currentManifest={preview.current_manifest}
            targetManifest={preview.target_manifest}
          />
          <Button
            type="button"
            data-testid="helm-upgrade-apply"
            disabled={busy}
            onClick={() => setConfirming(true)}
          >
            {applying ? 'Applying…' : 'Confirm upgrade'}
          </Button>
        </section>
      )}

      <ConfirmDialog
        open={confirming}
        title="Upgrade Helm release"
        items={[{ namespace, name: release }]}
        detail={
          preview?.gitops_warning
            ? `This release is managed by ${preview.gitops_warning.controller}; a manual upgrade may be reverted or conflict with GitOps reconciliation.`
            : 'This runs a real `helm upgrade` against the cluster.'
        }
        confirmLabel="Upgrade"
        onConfirm={onConfirmApply}
        onCancel={() => setConfirming(false)}
      />
    </div>
  )
}
