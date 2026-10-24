/**
 * 051 FR-015: the server's compose lint while editing. A `LintCompose` for the text once typing
 * pauses; only the reply to the latest request is kept, so a slow answer never shows findings for
 * text that is gone. Warnings only: saving is gated by the server, not by this.
 */
import { useEffect, useRef, useState } from 'react'

import { onServerEvent, sendCmd } from '@/app/connection'
import type { ComposeWarning, RequestId } from '@/protocol/generated'

/** How long typing pauses before the text is linted. */
export const LINT_DELAY_MS = 400

export function useComposeLint(stack: string, text: string): ComposeWarning[] {
  const [findings, setFindings] = useState<ComposeWarning[]>([])
  const latest = useRef<RequestId | null>(null)

  useEffect(
    () =>
      onServerEvent(({ reply_to, event }) => {
        if (event.type !== 'LintResult' || reply_to === undefined || reply_to !== latest.current) return
        setFindings(event.data.findings)
      }),
    [],
  )

  useEffect(() => {
    const t = setTimeout(() => {
      latest.current = sendCmd({ type: 'LintCompose', data: { stack, compose_text: text } })
    }, LINT_DELAY_MS)
    return () => clearTimeout(t)
  }, [stack, text])

  return findings
}
