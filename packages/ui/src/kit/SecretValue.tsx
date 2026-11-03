/**
 * A stored secret's value (spec 059): never in the page until someone asks. Reveal calls `reveal`,
 * where the caller does its step-up and fetches the value; resolving `null` means they backed out
 * and nothing changes. The value then shows with Copy and Hide, and hides itself again after
 * `hideAfterMs`. The mask is a fixed length, so it does not give the value's length away.
 */
import { CheckIcon, CopyIcon, EyeIcon, EyeSlashIcon } from '@phosphor-icons/react'
import { useEffect, useId, useState } from 'react'

import { Button } from '../button'
import { copyText } from './clipboard'

const COPIED_MS = 2000
const MASK = '•'.repeat(16)

type Shown = { kind: 'hidden' } | { kind: 'revealing' } | { kind: 'shown'; value: string } | { kind: 'failed'; message: string }

export function SecretValue({
  label,
  reveal,
  hideAfterMs = 30_000,
  data,
}: {
  label: string
  /** Steps up if it must and fetches the value; `null` when the person cancelled. */
  reveal: () => Promise<string | null>
  hideAfterMs?: number
  /** `data-secret` on the value, for tests. */
  data?: string
}) {
  const [shown, setShown] = useState<Shown>({ kind: 'hidden' })
  const [copied, setCopied] = useState(false)
  const id = useId()
  const name = label.toLowerCase()

  useEffect(() => {
    if (shown.kind !== 'shown') return
    const t = setTimeout(() => {
      setShown({ kind: 'hidden' })
    }, hideAfterMs)
    return () => {
      clearTimeout(t)
    }
  }, [shown, hideAfterMs])

  useEffect(() => {
    if (!copied) return
    const t = setTimeout(() => {
      setCopied(false)
    }, COPIED_MS)
    return () => {
      clearTimeout(t)
    }
  }, [copied])

  const onReveal = () => {
    setShown({ kind: 'revealing' })
    reveal().then(
      (value) => {
        setShown(value === null ? { kind: 'hidden' } : { kind: 'shown', value })
      },
      (e: unknown) => {
        setShown({ kind: 'failed', message: e instanceof Error ? e.message : String(e) })
      },
    )
  }

  const value = shown.kind === 'shown' ? shown.value : null
  const seconds = Math.round(hideAfterMs / 1000)
  return (
    <div className="flex min-w-0 flex-col gap-1" data-secret-block={data}>
      <div className="flex min-w-0 items-center gap-1">
        <span id={id} className="min-w-0 flex-1 truncate text-xs text-muted-foreground">
          {label}
        </span>
        {value === null ? (
          <Button
            type="button"
            variant="outline"
            size="xs"
            aria-label={`Reveal ${name}`}
            disabled={shown.kind === 'revealing'}
            onClick={onReveal}
          >
            <EyeIcon aria-hidden />
            {shown.kind === 'revealing' ? 'Revealing…' : 'Reveal'}
          </Button>
        ) : (
          <>
            <Button
              type="button"
              variant="ghost"
              size="xs"
              aria-label={`Hide ${name}`}
              onClick={() => {
                setShown({ kind: 'hidden' })
              }}
            >
              <EyeSlashIcon aria-hidden />
              Hide
            </Button>
            <Button
              type="button"
              variant="outline"
              size="xs"
              aria-label={`Copy ${name}`}
              onClick={() => {
                void copyText(value).then(setCopied)
              }}
            >
              {copied ? <CheckIcon aria-hidden className="text-status-ok" /> : <CopyIcon aria-hidden />}
              {copied ? 'Copied' : 'Copy'}
            </Button>
          </>
        )}
        <span role="status" className="sr-only">
          {copied ? `${label} copied` : value === null ? '' : `${label} shown; it hides after ${String(seconds)} seconds`}
        </span>
      </div>
      <pre
        aria-labelledby={id}
        data-secret={data}
        data-revealed={value !== null}
        className="max-h-48 min-w-0 overflow-y-auto border border-border bg-muted/30 p-2 font-mono text-xs whitespace-pre-wrap break-all"
      >
        {value ?? MASK}
      </pre>
      {shown.kind === 'failed' && (
        <p role="alert" className="text-xs text-status-err">
          Could not reveal the {name}: {shown.message}
        </p>
      )}
    </div>
  )
}
