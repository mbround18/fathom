/**
 * Local port picker for the port-forward start flow (009, US1/US3). Controlled port input:
 * dispatches a debounced `CheckPortAvailability` on edit, reads the result from
 * `integrations.portAvailability` (already populated by `store/handlers/system.ts`'s
 * `PortAvailability` case), and renders an inline conflict warning (FR-006).
 *
 * The debounce (300ms, data-model.md) applies to *dispatch*, not to the input value itself — the
 * field always reflects what the operator typed immediately.
 */
import { useEffect, useRef, useState } from 'react'

import { Input } from '@companion/ui'
import { useStore } from '@/store'

const DEBOUNCE_MS = 300

export interface PortForwardPortFieldProps {
  /** Current chosen local port. */
  value: number
  onChange: (port: number) => void
  /** Reported to the parent so it can gate its confirm action (US3). */
  onConflictChange?: (conflict: boolean) => void
  id?: string
}

export function PortForwardPortField({
  value,
  onChange,
  onConflictChange,
  id,
}: PortForwardPortFieldProps) {
  const send = useStore((s) => s.send)
  const availability = useStore((s) => s.integrations.portAvailability)
  const [text, setText] = useState(() => String(value))
  const timer = useRef<ReturnType<typeof setTimeout>>(undefined)

  // Keep the field in sync when the caller changes `value` programmatically (e.g. restart
  // seeding the previous local_port, or the start dialog's suggested-port reset) — adjusted
  // during render rather than an effect (react.dev "Adjusting state when a prop changes").
  const [prevValue, setPrevValue] = useState(value)
  if (value !== prevValue) {
    setPrevValue(value)
    setText(String(value))
  }

  const dispatchCheck = (port: number) => {
    if (timer.current) clearTimeout(timer.current)
    timer.current = setTimeout(() => {
      send({ type: 'CheckPortAvailability', data: { port } })
    }, DEBOUNCE_MS)
  }

  useEffect(
    () => () => {
      if (timer.current) clearTimeout(timer.current)
    },
    [],
  )

  const conflict = availability[value] === false
  useEffect(() => {
    onConflictChange?.(conflict)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [conflict])

  return (
    <div className="flex flex-col gap-1">
      <label htmlFor={id} className="text-xs text-muted-foreground">
        Local port
      </label>
      <Input
        id={id}
        data-testid="port-forward-local-port"
        inputMode="numeric"
        autoComplete="off"
        value={text}
        onChange={(e) => {
          const next = e.target.value
          setText(next)
          const port = Number.parseInt(next, 10)
          if (Number.isFinite(port) && port > 0 && port < 65536) {
            onChange(port)
            dispatchCheck(port)
          }
        }}
        className="w-32 font-mono"
      />
      {conflict && (
        <p data-testid="port-forward-port-conflict" className="text-xs text-status-err">
          Port {value} is already in use on this machine.
        </p>
      )}
    </div>
  )
}
