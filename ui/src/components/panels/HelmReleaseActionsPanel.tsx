/**
 * Helm release actions tab (006-helm-mutate US3, T034): "Uninstall" only for now — rollback
 * (per revision row) and upgrade live in `helm.tsx`'s own detail sections respectively once
 * US1/US2 land (tasks.md T018/T027). Uninstall has no diff-preview step (nothing to preview,
 * per spec.md's US3 rationale): the button goes straight to the GitOps-warning-aware
 * `ConfirmDialog` and, on confirm, dispatches `UninstallHelmRelease`.
 *
 * The GitOps-warning banner reuses the shared `GitOpsWarningBanner` component (contracts/
 * protocol.md's T019 banner: "reusing 005's origin-link rendering from `helm.tsx`") rather than
 * a second copy — same pattern as rollback (`resources/helm.tsx`) and upgrade
 * (`HelmUpgradePanel.tsx`).
 *
 * Outcome comes from `HelmMutationApplied`/`HelmMutationFailed` (`store/handlers/system.ts`),
 * not the generic `CommandFinished` ack — those are the events that carry the actual mutate-flow
 * result, and a success also closes this Inspector (the release is gone), which is why this
 * panel does not itself navigate away on success.
 */
import { TrashIcon } from '@phosphor-icons/react'
import { useEffect, useState } from 'react'
import { toast } from 'sonner'

import { ConfirmDialog } from '@/components/ConfirmDialog'
import { Button } from '@companion/ui'
import { Gated } from '@/lib/capabilities'
import { GitOpsWarningBanner } from '@/components/panels/GitOpsWarningBanner'
import type { GitOpsOriginRef } from '@/protocol/types'
import { useStore } from '@/store'

const ACTION_TOAST_TEST_ID = 'helm-uninstall-toast'

export function HelmReleaseActionsPanel({
  namespace,
  release,
  revision,
  origin,
}: {
  namespace: string
  release: string
  revision: number
  origin: GitOpsOriginRef | null
}) {
  const request = useStore((s) => s.request)
  const setHelmUninstall = useStore((s) => s.setHelmUninstall)
  const key = `${namespace}/${release}`
  const uninstall = useStore((s) => s.helmUninstalls[key])
  const [confirming, setConfirming] = useState(false)

  const busy = uninstall?.status === 'in-progress'

  // React to the outcome events (`store/handlers/system.ts`) rather than `CommandFinished`: a
  // successful uninstall closes this Inspector from there, so the toast is this panel's only
  // remaining job on success; on failure the panel stays open so the operator can retry.
  useEffect(() => {
    if (uninstall?.status === 'complete') {
      toast.success(`✓ Uninstalled ${namespace}/${release}`, { testId: ACTION_TOAST_TEST_ID })
      setHelmUninstall(key, undefined)
    } else if (uninstall?.status === 'failed') {
      toast.error(`✖ Failed to uninstall ${namespace}/${release}`, {
        testId: ACTION_TOAST_TEST_ID,
        description: uninstall.error,
        duration: Infinity,
      })
    }
    // Only fire once per transition into a terminal state.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [uninstall?.status])

  const onConfirm = () => {
    setConfirming(false)
    setHelmUninstall(key, { status: 'in-progress' })
    void request({
      type: 'UninstallHelmRelease',
      data: { namespace, release, base_revision: revision },
    }).catch((err: unknown) => {
      setHelmUninstall(key, {
        status: 'failed',
        error: err instanceof Error ? err.message : String(err),
      })
    })
  }

  return (
    <div className="flex h-full min-h-0 flex-col gap-4 overflow-auto p-3">
      <section className="flex flex-col gap-2">
        <h3 className="font-display text-xs font-semibold tracking-wider text-accent-neon uppercase">
          Uninstall
        </h3>
        <p className="text-xs text-muted-foreground">
          Removes this release&apos;s Helm release-storage record and its installed resources. This
          cannot be undone.
        </p>
        <div>
          <Gated verb="delete" resource="secrets" namespace={namespace}>
            <Button
              type="button"
              variant="destructive"
              size="sm"
              disabled={busy}
              data-testid="helm-release-uninstall"
              onClick={() => setConfirming(true)}
            >
              <TrashIcon data-icon="inline-start" /> Uninstall release
            </Button>
          </Gated>
        </div>
        {uninstall?.status === 'failed' && (
          <p data-testid="helm-uninstall-error" className="text-xs text-status-err">
            {uninstall.error}
          </p>
        )}
      </section>

      <ConfirmDialog
        open={confirming}
        title="Uninstall release"
        items={[{ namespace, name: release }]}
        detail={
          <div className="flex flex-col gap-2">
            <span>
              This uninstalls the release and deletes the resources it installed. This cannot be
              undone.
            </span>
            {origin && (
              <GitOpsWarningBanner
                origin={origin}
                testId="helm-uninstall-gitops-warning"
                consequence="Uninstalling here does not stop that controller from re-creating it, unless the source is also removed."
              />
            )}
          </div>
        }
        destructive
        requireTypedCount={false}
        confirmLabel="Uninstall"
        onConfirm={onConfirm}
        onCancel={() => setConfirming(false)}
      />
    </div>
  )
}
