/**
 * What became of a change Keel proposed to the repository (spec 066 `ChangeOutcome`): a pull
 * request it opened, a patch to apply by hand when there is no forge connection, or the forge's
 * refusal with the same patch. Shared by Draft job, Add host and Propose sops recipient.
 */
import { ArrowSquareOutIcon } from '@phosphor-icons/react'
import { CopyBlock, LINK } from '@companion/ui/kit'

import type { ChangeOutcome } from '@/protocol/wire'

export function ChangeOutcomeView({ outcome }: { outcome: ChangeOutcome }) {
  return (
    <div data-testid="change-outcome" data-outcome={outcome.kind} className="flex min-w-0 flex-col gap-2 text-xs">
      {outcome.kind === 'opened' && (
        <p className="min-w-0 break-all">
          <span className="font-medium text-status-ok">Pull request opened</span> on branch <code className="font-mono">{outcome.branch}</code>:{' '}
          <a href={outcome.url} target="_blank" rel="noreferrer" className={`${LINK} inline-flex items-center gap-1 break-all`}>
            {outcome.url}
            <ArrowSquareOutIcon aria-hidden className="size-3 shrink-0" />
          </a>
        </p>
      )}
      {outcome.kind === 'manual' && (
        <>
          <p className="min-w-0 break-words">
            <span className="font-medium">Apply this change by hand.</span> {outcome.reason}
          </p>
          <CopyBlock label="Patch" value={outcome.diff} data="change-diff" />
        </>
      )}
      {outcome.kind === 'failed' && (
        <>
          <p role="alert" className="min-w-0 break-words text-status-err">
            The forge refused the change: {outcome.error}
          </p>
          <CopyBlock label="Patch to apply by hand" value={outcome.diff} data="change-diff" />
        </>
      )}
    </div>
  )
}
