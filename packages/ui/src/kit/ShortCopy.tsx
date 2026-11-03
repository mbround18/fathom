/**
 * A long identifier shown short (Keel's build digests and revisions, credential fingerprints): the
 * start and the end in monospace, the full value in the title, and a Copy button for the full
 * value. People compare fingerprints by their ends, so the middle is what goes.
 */
import { CheckIcon, CopyIcon } from '@phosphor-icons/react'
import { useEffect, useState } from 'react'

import { Button } from '../button'
import { copyText } from './clipboard'

const COPIED_MS = 2000
const ALGORITHM = /^(sha1|sha224|sha256|sha384|sha512|md5|blake2b|blake3):/i

/** `keep` characters from each end with an ellipsis between; an algorithm prefix stays whole. */
export function shorten(value: string, keep = 8): string {
  const prefix = ALGORITHM.exec(value)?.[0] ?? ''
  const body = value.slice(prefix.length)
  if (body.length <= keep * 2 + 1) return value
  return `${prefix}${body.slice(0, keep)}…${body.slice(-keep)}`
}

export function ShortCopy({
  label,
  value,
  short = shorten(value),
}: {
  /** What the value is, lower case: "digest", "fingerprint". */
  label: string
  value: string
  short?: string
}) {
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

/** A key's or a secret version's fingerprint: never the value, only what identifies it. */
export function Fingerprint({ value, label = 'fingerprint' }: { value: string; label?: string }) {
  return <ShortCopy label={label} value={value} />
}
