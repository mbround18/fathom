/**
 * Secret key list. SECURITY (constitution: user data sacred): values are never rendered by
 * default and never travel with the Secret's details (`SecretDetails` carries key names only
 * and `raw_json` is redacted by the backend). "Reveal" fetches one key's value on demand
 * (`RevealSecretValue`, store/secretReveal.ts); the value lives only in this component's state,
 * never in the store or any cache. Revealed keys re-mask (and the values are dropped) when
 * focus leaves the section, on Escape, on Hide, and on unmount (inspector close). Values are
 * never logged or persisted, and copy is offered only while a value is shown.
 */
import { useEffect, useRef, useState } from 'react'

import { Button } from '@/components/ui/button'
import { revealSecretValue, type RevealResult } from '@/store/secretReveal'

const MASK = '••••••••'

type KeyState = { status: 'loading' } | { status: 'done'; result: RevealResult }

/** Fetches one key's value; injectable for tests. */
export type RevealFn = (key: string) => Promise<RevealResult>

/** What to show for a finished reveal (the value itself only for `Value`). */
function revealText(result: RevealResult): string {
  switch (result.type) {
    case 'Value':
      return result.data.value === '' ? '(empty value)' : result.data.value
    case 'Binary':
      return `Binary value (${String(result.data.size)} bytes), not shown as text`
    case 'KeyMissing':
      return 'This key is no longer in the Secret'
    case 'NotFound':
      return 'Not found: this Secret no longer exists'
    case 'Forbidden':
      return `Forbidden: you are not allowed to read this Secret's values (${result.data.message})`
    case 'Failed':
      return `Could not read the value: ${result.data.message}`
    case 'NoAnswer':
      return `Could not read the value: ${result.data.message}`
  }
}

export function SecretKeys({
  keys,
  namespace,
  name,
  reveal,
}: {
  keys: string[]
  namespace: string
  name: string
  reveal?: RevealFn
}) {
  const [shown, setShown] = useState<Readonly<Record<string, KeyState>>>({})
  // Each reveal carries a ticket; an answer whose ticket was cancelled (hidden, re-masked,
  // unmounted, or another Secret selected) is discarded, never shown.
  const tickets = useRef(new Map<string, number>())
  const nextTicket = useRef(0)

  const remask = () => {
    tickets.current.clear()
    setShown((prev) => (Object.keys(prev).length === 0 ? prev : {}))
  }

  // A different Secret: nothing revealed for the previous one may linger.
  useEffect(() => {
    const pending = tickets.current
    return () => {
      pending.clear()
      setShown({})
    }
  }, [namespace, name])

  const hide = (k: string) => {
    tickets.current.delete(k)
    setShown((prev) => Object.fromEntries(Object.entries(prev).filter(([key]) => key !== k)))
  }

  const show = (k: string) => {
    const ticket = ++nextTicket.current
    tickets.current.set(k, ticket)
    setShown((prev) => ({ ...prev, [k]: { status: 'loading' } }))
    const fetchValue = reveal ?? ((key: string) => revealSecretValue(namespace, name, key))
    void fetchValue(k).then((result) => {
      if (tickets.current.get(k) !== ticket) return
      setShown((prev) => ({ ...prev, [k]: { status: 'done', result } }))
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
        const state = shown[k]
        const result = state?.status === 'done' ? state.result : undefined
        const value = result?.type === 'Value' ? result.data.value : undefined
        const failed = result !== undefined && result.type !== 'Value'
        return (
          <div
            key={k}
            data-testid={`secret-key-${k}`}
            className="flex flex-wrap items-center gap-2"
          >
            <span className="font-mono">{k}</span>
            <span
              data-testid={`secret-value-${k}`}
              role={failed ? 'alert' : undefined}
              aria-busy={state?.status === 'loading' || undefined}
              className={
                failed
                  ? 'min-w-0 break-all text-status-warn'
                  : 'min-w-0 break-all text-muted-foreground'
              }
            >
              {state === undefined ? MASK : result ? revealText(result) : 'Revealing…'}
            </span>
            <Button
              type="button"
              size="sm"
              variant="outline"
              aria-pressed={state !== undefined}
              aria-label={`${state ? 'Hide' : 'Reveal'} value of ${k}`}
              onClick={() => {
                if (state) hide(k)
                else show(k)
              }}
            >
              {state ? 'Hide' : 'Reveal'}
            </Button>
            {value !== undefined && (
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
