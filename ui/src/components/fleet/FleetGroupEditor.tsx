/**
 * 046-fleet-model US2 (T036): define a group with the expression you already type.
 *
 * A group is a name and a label selector — one text field holding what the operator would hand
 * `kubectl -l`, not a `matchLabels`/`matchExpressions` form builder. Principle I rules out exactly
 * that: a wizard over a Kubernetes concept the operator already knows, which would also have to
 * hold its own opinion of the grammar.
 *
 * This editor parses nothing. `src/backend/fleet_model.rs` is the only parser (Principle II), so
 * the expression is sent verbatim as typed and a refusal arrives as `FleetModelRejected
 * { reason, field }` whose `reason` is rendered word for word. The fields clear only once the
 * backend reports the group in `FleetModelUpdated` — until then nothing is assumed saved, and on a
 * refusal the draft stays on screen to be corrected (FR-006, Principle I).
 */
import { useState } from 'react'

import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { useStore } from '@/store'

/** The `FleetModelRejected.field` values that belong to this editor. */
const FIELDS: readonly string[] = ['selector', 'name', 'group']

export function FleetGroupEditor() {
  const send = useStore((s) => s.send)
  const groups = useStore((s) => s.session.fleetGroups)
  const rejection = useStore((s) => s.session.fleetRejection)

  const [name, setName] = useState('')
  const [selector, setSelector] = useState('')
  /** The name awaiting the backend's verdict. */
  const [awaiting, setAwaiting] = useState<string | null>(null)

  // Success is the backend saying so, never this component assuming so (Principle I). Derived at
  // render rather than written back by an effect, which would only cascade a second render.
  const saved = awaiting !== null && groups.some((g) => g.name === awaiting)
  const draftName = saved ? '' : name
  const draftSelector = saved ? '' : selector

  /** Editing after a save starts a fresh draft rather than reviving the saved one. */
  const edit = (next: { name?: string; selector?: string }) => {
    if (saved) setAwaiting(null)
    setName(next.name ?? (saved ? '' : name))
    setSelector(next.selector ?? (saved ? '' : selector))
  }

  const refusal =
    rejection !== null && (rejection.field === null || FIELDS.includes(rejection.field))
      ? rejection
      : null

  const save = () => {
    // Sent as typed: an empty name or an unparseable expression is the backend's to refuse.
    send({ type: 'SaveFleetGroup', data: { name: draftName.trim(), selector: draftSelector } })
    setAwaiting(draftName.trim())
  }

  return (
    <form
      data-testid="fleet-group-editor"
      className="flex flex-col gap-2 rounded-md border border-border/60 bg-background/30 p-2"
      onSubmit={(e) => {
        e.preventDefault()
        save()
      }}
    >
      <div className="flex flex-col gap-2 sm:flex-row sm:items-center">
        <Input
          data-testid="fleet-group-name-input"
          aria-label="Group name"
          placeholder="prod"
          className="sm:w-40"
          value={draftName}
          onChange={(e) => edit({ name: e.target.value })}
        />
        <Input
          data-testid="fleet-group-selector-input"
          aria-label="Label selector"
          placeholder="env=prod, region in (us-east-1, us-west-2)"
          className="font-mono sm:flex-1"
          value={draftSelector}
          onChange={(e) => edit({ selector: e.target.value })}
        />
        <Button data-testid="fleet-group-save" type="submit" size="sm" className="sm:w-auto">
          Save group
        </Button>
      </div>
      {refusal === null ? null : (
        <p
          data-testid="fleet-group-error"
          data-field={refusal.field ?? ''}
          className="text-xs break-words text-red-400"
        >
          {refusal.reason}
        </p>
      )}
    </form>
  )
}
