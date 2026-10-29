/**
 * What an update plan tells a person besides its steps, shared by the batch preview and the stack
 * page's Update.
 *
 * - `RollbackNotes`: the services whose old image may not come back if the update fails (it was
 *   pulled by hand and has no tag left). The update still goes ahead once confirmed.
 * - `UpToDate`: nothing to recreate. Information, not an error, with each service and why it is
 *   left as it is.
 */
import type { ComposeWarning, StackUpdatePlan } from '@/protocol/generated'

import { rollbackNotes } from './model'

export function RollbackNotes({ findings }: { findings: readonly ComposeWarning[] }) {
  const notes = rollbackNotes(findings)
  if (notes.length === 0) return null
  return (
    <ul data-field="rollback-notes" aria-label="If the update fails" className="flex flex-col gap-0.5 text-xs text-status-warn">
      {notes.map((n) => (
        <li key={n.service} data-rollback-note={n.service} className="min-w-0 break-words">
          {n.service}: {n.message}
        </li>
      ))}
    </ul>
  )
}

/** `named` prefixes each service with its stack (`stack/service`): a batch lists several stacks. */
export function UpToDate({ stacks, named = false }: { stacks: readonly StackUpdatePlan[]; named?: boolean }) {
  return (
    <div className="flex flex-col gap-1 text-xs">
      <p role="status" data-up-to-date>
        Everything is up to date.
      </p>
      <ul aria-label="Left as they are" className="flex flex-col gap-0.5 text-muted-foreground">
        {stacks.flatMap((p) =>
          p.skipped.map(([service, why]) => (
            <li key={`${p.stack}/${service}`} data-plan-skipped={`${p.stack}/${service}`} className="min-w-0 break-words">
              {named ? `${p.stack}/` : ''}
              {service}: {why}
            </li>
          )),
        )}
      </ul>
    </div>
  )
}
