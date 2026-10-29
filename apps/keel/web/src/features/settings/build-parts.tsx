/**
 * Pieces the build panel and the update card share (spec 061): a UTC time, and a short value with
 * the full one in its title and on Copy.
 */
import { CheckIcon, CopyIcon } from '@phosphor-icons/react'
import { Button } from '@companion/ui'
import { useEffect, useState, type ReactNode } from 'react'

import { copyText } from '@/ui/clipboard'

import { when } from './updates'

const COPIED_MS = 2000

export function Time({ at }: { at: string }) {
  return (
    <time dateTime={at} title={at} className="tabular-nums">
      {when(at)}
    </time>
  )
}

/** `short` in monospace, the full `value` on hover, and a Copy button for the full value. */
export function ShortCopy({ label, short, value }: { label: string; short: string; value: string }) {
  const [copied, setCopied] = useState(false)
  useEffect(() => {
    if (!copied) return
    const t = setTimeout(() => {
      setCopied(false)
    }, COPIED_MS)
    return () => {
      clearTimeout(t)
    }
  }, [copied])
  return (
    <span className="inline-flex min-w-0 flex-wrap items-center gap-1">
      <code title={value} data-full={value} className="min-w-0 font-mono break-all">
        {short}
      </code>
      <Button
        type="button"
        variant="ghost"
        size="xs"
        className="text-muted-foreground hover:text-foreground"
        title={`Copy full ${label}`}
        aria-label={`Copy full ${label}`}
        onClick={() => {
          void copyText(value).then(setCopied)
        }}
      >
        {copied ? <CheckIcon aria-hidden className="text-status-ok" /> : <CopyIcon aria-hidden />}
      </Button>
      <span role="status" className="sr-only">
        {copied ? `Full ${label} copied` : ''}
      </span>
    </span>
  )
}

/** A value Keel doesn't have, said plainly and quieter than a real one. */
export function Unknown({ text = 'unknown' }: { text?: string }) {
  return <span className="text-muted-foreground">{text}</span>
}

/**
 * A definition list's row. Every value sits in a row as tall as the Copy button, so rows with and
 * without one line up on the same rhythm.
 */
export function Row({ term, children, data }: { term: string; children: ReactNode; data?: string }) {
  return (
    <>
      <dt className="flex min-h-6 items-center self-start text-muted-foreground">{term}</dt>
      <dd data-field={data} className="flex min-h-6 min-w-0 items-center">
        {children}
      </dd>
    </>
  )
}
