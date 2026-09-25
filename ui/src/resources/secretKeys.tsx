/**
 * Secret key list. SECURITY (constitution: user data sacred): values are never rendered by
 * default. Each key shows its name and a masked placeholder; "Reveal" is a per-key, local-display
 * only toggle. Revealed keys re-mask when focus leaves the section, on Escape, and on unmount
 * (inspector close). Values are never logged or persisted, and copy is offered only after reveal.
 *
 * Protocol v1 never transfers secret values (`SecretDetails` carries key names only and
 * `raw_json` is redacted by the backend), so `values` is normally undefined and Reveal explains
 * that. The component honours `values` if a future, explicitly authorised source supplies them.
 */
import { useState } from 'react'

import { Button } from '@/components/ui/button'

const MASK = '••••••••'

export function SecretKeys({
  keys,
  values,
}: {
  keys: string[]
  values?: Record<string, string>
}) {
  const [revealed, setRevealed] = useState<ReadonlySet<string>>(new Set())
  const remask = () => {
    setRevealed((prev) => (prev.size === 0 ? prev : new Set()))
  }
  const toggle = (k: string) => {
    setRevealed((prev) => {
      const next = new Set(prev)
      if (next.has(k)) next.delete(k)
      else next.add(k)
      return next
    })
  }
  if (keys.length === 0) return <span className="text-muted-foreground">no keys</span>
  return (
    // Focus/Escape listeners only re-mask; the interactive controls are the child buttons.
    // eslint-disable-next-line jsx-a11y/no-noninteractive-element-interactions
    <div
      data-testid="secret-keys"
      role="group"
      aria-label="Secret keys"
      className="flex flex-col gap-1"
      onBlur={(e) => {
        if (!e.currentTarget.contains(e.relatedTarget)) remask()
      }}
      onKeyDown={(e) => {
        if (e.key === 'Escape') remask()
      }}
    >
      {keys.map((k) => {
        const shown = revealed.has(k)
        const value = values?.[k]
        return (
          <div key={k} data-testid={`secret-key-${k}`} className="flex flex-wrap items-center gap-2">
            <span className="font-mono">{k}</span>
            <span
              data-testid={`secret-value-${k}`}
              className="min-w-0 break-all text-muted-foreground"
            >
              {shown
                ? (value ?? 'Value not transferred: the backend never sends secret values')
                : MASK}
            </span>
            <Button
              type="button"
              size="sm"
              variant="outline"
              aria-pressed={shown}
              aria-label={`${shown ? 'Hide' : 'Reveal'} value of ${k}`}
              onClick={() => {
                toggle(k)
              }}
            >
              {shown ? 'Hide' : 'Reveal'}
            </Button>
            {shown && value !== undefined && (
              <Button
                type="button"
                size="sm"
                variant="outline"
                aria-label={`Copy value of ${k}`}
                onClick={() => {
                  void navigator.clipboard.writeText(value)
                }}
              >
                Copy
              </Button>
            )}
          </div>
        )
      })}
    </div>
  )
}
