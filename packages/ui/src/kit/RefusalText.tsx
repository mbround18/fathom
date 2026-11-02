/**
 * A `Denied` or `OpError`, verbatim: the action (when the app's denial names one) and the reason
 * code, or the server's message and
 * its code. The app not being connected is not the server's answer, so it reads as a plain sentence.
 * `lead` goes in front, in the same alert, so it is read out with the reason.
 */
import { WarningOctagonIcon } from '@phosphor-icons/react'
import type { ReactNode } from 'react'

import { isTransportFailure, type Refusal } from './refusal'

export function RefusalText({ refusal, lead }: { refusal: Refusal; lead?: ReactNode }) {
  return (
    <p
      role="alert"
      data-refusal={refusal.kind}
      className="flex min-w-0 items-start gap-1.5 border border-status-err/40 px-2 py-1.5 text-sm text-status-err"
    >
      <WarningOctagonIcon aria-hidden className="mt-0.5 size-4 shrink-0" />
      <span className="min-w-0 break-words">
        {lead !== undefined && <span className="font-medium">{lead} </span>}
        {refusal.kind === 'denied' ? (
          refusal.action === undefined ? (
            <>
              Denied (<code data-denied-reason>{refusal.reason}</code>)
            </>
          ) : (
            <>
              Denied: <code data-denied-action>{refusal.action}</code> (
              <code data-denied-reason>{refusal.reason}</code>)
            </>
          )
        ) : isTransportFailure(refusal) ? (
          <span className="whitespace-pre-wrap">{refusal.message}</span>
        ) : (
          <>
            <span className="whitespace-pre-wrap">{refusal.message}</span> (<code data-error-code>{refusal.code}</code>)
          </>
        )}
      </span>
    </p>
  )
}
