/**
 * A value to copy (spec 060 R10): a label, the value in monospace, and a Copy button that says
 * "Copied" for a moment. `masked` hides the value behind dots until Reveal; Copy always copies the
 * real value. Long values wrap rather than scroll the page.
 */
import { CheckIcon, CopyIcon, EyeIcon, EyeSlashIcon } from '@phosphor-icons/react'
import { Button } from '../button'
import { useEffect, useId, useState, type ReactNode } from 'react'

import { copyText } from './clipboard'

const COPIED_MS = 2000

export function CopyBlock({
  label,
  tip,
  value,
  masked = false,
  data,
  valueData,
}: {
  label: string
  /** An info icon after the label (a glossary term's `TermTip`). */
  tip?: ReactNode
  value: string
  masked?: boolean
  /** `data-copy` on the value, for tests. */
  data?: string
  /** More `data-*` attributes on the value element, for hooks older than `data-copy`. */
  valueData?: Record<`data-${string}`, string | boolean>
}) {
  const [revealed, setRevealed] = useState(!masked)
  const [copied, setCopied] = useState(false)
  const id = useId()

  useEffect(() => {
    if (!copied) return
    const t = setTimeout(() => {
      setCopied(false)
    }, COPIED_MS)
    return () => {
      clearTimeout(t)
    }
  }, [copied])

  const shown = revealed ? value : '•'.repeat(Math.min(value.length, 32))
  return (
    <div className="flex min-w-0 flex-col gap-1" data-copy-block={data}>
      <div className="flex min-w-0 items-center gap-1">
        <span id={id} className={`min-w-0 truncate text-xs text-muted-foreground ${tip === undefined ? 'flex-1' : ''}`}>
          {label}
        </span>
        {tip !== undefined && <span className="flex flex-1 items-center pl-1.5">{tip}</span>}
        {masked && (
          <Button
            type="button"
            variant="ghost"
            size="xs"
            aria-pressed={revealed}
            aria-label={revealed ? `Hide ${label.toLowerCase()}` : `Reveal ${label.toLowerCase()}`}
            onClick={() => {
              setRevealed((r) => !r)
            }}
          >
            {revealed ? <EyeSlashIcon aria-hidden /> : <EyeIcon aria-hidden />}
            {revealed ? 'Hide' : 'Reveal'}
          </Button>
        )}
        <Button
          type="button"
          variant="outline"
          size="xs"
          aria-label={`Copy ${label.toLowerCase()}`}
          onClick={() => {
            void copyText(value).then(setCopied)
          }}
        >
          {copied ? <CheckIcon aria-hidden className="text-status-ok" /> : <CopyIcon aria-hidden />}
          {copied ? 'Copied' : 'Copy'}
        </Button>
        <span role="status" className="sr-only">
          {copied ? `${label} copied` : ''}
        </span>
      </div>
      <pre
        {...valueData}
        aria-labelledby={id}
        data-copy={data}
        data-masked={masked && !revealed}
        className="max-h-48 min-w-0 overflow-y-auto border border-border bg-muted/30 p-2 font-mono text-xs whitespace-pre-wrap break-all select-all"
      >
        {shown}
      </pre>
    </div>
  )
}
