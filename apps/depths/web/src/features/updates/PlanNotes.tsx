/**
 * What an update plan tells a person besides its steps, shared by the batch preview and the stack
 * page's Update.
 *
 * - `RollbackNotes`: the services whose old image may not come back if the update fails (it was
 *   pulled by hand and has no tag left). The update still goes ahead once confirmed.
 */
import type { ComposeWarning } from '@/protocol/generated'

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
