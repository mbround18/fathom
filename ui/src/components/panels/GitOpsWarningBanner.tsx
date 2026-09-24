/**
 * Shared GitOps-ownership warning banner (006-helm-mutate, contracts/protocol.md's T019 banner:
 * "reusing 005's origin-link rendering from `helm.tsx`, same link-out affordance"). Rollback
 * (`resources/helm.tsx`), upgrade (`HelmUpgradePanel.tsx`) and uninstall
 * (`HelmReleaseActionsPanel.tsx`) all show this when a mutate preview/action targets a
 * Flux-managed release — one implementation instead of three near-identical copies.
 */
import { WarningIcon } from '@phosphor-icons/react'

import { StatusBadge } from '@/components/StatusBadge'
import type { GitOpsOriginRef } from '@/protocol/types'
import { useStore } from '@/store'

export function GitOpsWarningBanner({
  origin,
  testId,
  consequence,
}: {
  origin: GitOpsOriginRef
  testId: string
  /** Action-specific sentence describing what a manual mutation risks (e.g. "A rollback may be
   * reverted by the next reconciliation."). */
  consequence: string
}) {
  const openInspector = useStore((s) => s.openInspector)
  return (
    <div
      data-testid={testId}
      className="flex flex-col gap-1 border border-status-warn/60 bg-status-warn/10 p-2 text-xs"
    >
      <p className="flex items-center gap-1.5 font-semibold text-status-warn">
        <WarningIcon aria-hidden="true" className="size-4 shrink-0" weight="bold" />
        Managed by {origin.controller}
      </p>
      <p className="text-muted-foreground">
        This release is reconciled by{' '}
        <button
          type="button"
          className="text-foreground hover:text-accent-neon hover:underline"
          onClick={() => {
            openInspector(
              {
                section: origin.section,
                kind: origin.kind,
                namespace: origin.namespace,
                name: origin.name,
              },
              undefined,
              true,
            )
          }}
        >
          {origin.kind} {origin.namespace}/{origin.name}
        </button>
        . {consequence} <StatusBadge status={origin.ready} />
      </p>
    </div>
  )
}
