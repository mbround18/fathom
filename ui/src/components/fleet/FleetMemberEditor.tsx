/**
 * 046-fleet-model US1 (T029): name and label one fleet member.
 *
 * This editor implements **no** label grammar and **no** trimming rule. `src/backend/fleet_model.rs`
 * owns both; a second copy in TypeScript could disagree with the one that actually writes
 * `fleet.json`, which is precisely what Constitution Principle II forbids. So the operator's input
 * is sent as typed, and a refusal arrives as `FleetModelRejected { reason, field }` whose `reason`
 * is rendered verbatim against the field it names.
 *
 * Nothing is hidden behind a wizard step (Principle I): both fields and the member's current labels
 * are on screen at once, and the removal of a label is one click.
 */
import { useState } from 'react'

import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import type { FleetMember } from '@/protocol/types'
import { useStore } from '@/store'

/** The `FleetModelRejected.field` values this editor can place a message against. */
const DISPLAY_NAME_FIELD = 'display_name'
/** Every label rejection names the key it refused: `labels/<key>` (data-model.md `origins`). */
const LABEL_FIELD_PREFIX = 'labels/'

/**
 * Does this rejection belong to *this* editor at all?
 *
 * `fleetRejection` is one session-wide slot, so a refusal from the group editor
 * (`field: "selector"`) arrives here too. Only `display_name` and `labels/<key>` are fields this
 * editor owns; anything else is someone else's refusal and must not be rendered against an input
 * the operator did not touch.
 */
function owned(field: string | null): boolean {
  return field === DISPLAY_NAME_FIELD || (field !== null && field.startsWith(LABEL_FIELD_PREFIX))
}

export function FleetMemberEditor({ context, member }: { context: string; member?: FleetMember }) {
  const send = useStore((s) => s.send)
  const rejection = useStore((s) => s.session.fleetRejection)

  const persisted = member?.labels ?? {}
  const [name, setName] = useState(member?.display_name ?? '')
  const [labels, setLabels] = useState<Record<string, string>>(persisted)
  const [key, setKey] = useState('')
  const [value, setValue] = useState('')

  // Only a rejection this editor owns is shown here. `labels/<key>` and a null field — a refusal
  // that names no input, e.g. the disk refusing the write — belong to the label block; a
  // `field: "selector"` refusal from the group editor belongs to neither and renders nowhere.
  const mine = rejection !== null && (rejection.field === null || owned(rejection.field))
  const nameRejected = mine && rejection.field === DISPLAY_NAME_FIELD
  const labelsRejected = mine && !nameRejected

  const message = (show: boolean) =>
    show && rejection !== null ? (
      <p
        data-testid="fleet-member-rejection"
        data-field={rejection.field ?? ''}
        className="text-xs break-words text-red-400"
      >
        {rejection.reason}
      </p>
    ) : null

  const add = () => {
    // Sent as typed — an empty or malformed key is the backend's to refuse, not ours.
    setLabels({ ...labels, [key]: value })
    setKey('')
    setValue('')
  }

  const remove = (k: string) =>
    setLabels(Object.fromEntries(Object.entries(labels).filter(([at]) => at !== k)))

  const entries = Object.entries(labels)

  return (
    <div
      data-testid="fleet-member-editor"
      className="flex min-w-0 flex-col gap-2 border-t border-border/60 p-3"
    >
      <div className="flex min-w-0 flex-col gap-1 text-xs text-muted-foreground">
        <span>Display name</span>
        <span className="flex min-w-0 items-center gap-1">
          <Input
            data-testid="fleet-display-name-input"
            aria-label={`Display name for ${context}`}
            aria-invalid={nameRejected || undefined}
            value={name}
            onChange={(e) => setName(e.target.value)}
            className="h-8 min-w-0 text-sm"
          />
          <Button
            data-testid="fleet-display-name-save"
            size="sm"
            variant="outline"
            onClick={() =>
              send({
                type: 'SetFleetMemberDisplayName',
                data: { context, display_name: name === '' ? null : name },
              })
            }
          >
            Save
          </Button>
        </span>
      </div>
      {message(nameRejected)}

      <span className="text-xs text-muted-foreground">Labels</span>
      {entries.length > 0 && (
        <ul data-testid="fleet-editor-labels" className="flex flex-wrap gap-1">
          {entries.map(([k, v]) => (
            <li key={k} className="flex items-center gap-1">
              <span
                data-testid="fleet-editor-label"
                data-label-key={k}
                className="rounded border border-border/60 px-1.5 py-0.5 font-mono text-xs"
              >
                {k}={v}
              </span>
              <button
                type="button"
                data-testid="fleet-editor-label-remove"
                aria-label={`Remove label ${k}`}
                onClick={() => remove(k)}
                className="text-xs text-muted-foreground hover:text-foreground"
              >
                ×
              </button>
            </li>
          ))}
        </ul>
      )}
      <span className="flex min-w-0 flex-wrap items-center gap-1">
        <Input
          data-testid="fleet-label-key-input"
          aria-label={`Label key for ${context}`}
          placeholder="env"
          value={key}
          onChange={(e) => setKey(e.target.value)}
          className="h-8 min-w-0 flex-1 font-mono text-sm"
        />
        <Input
          data-testid="fleet-label-value-input"
          aria-label={`Label value for ${context}`}
          placeholder="prod"
          value={value}
          onChange={(e) => setValue(e.target.value)}
          className="h-8 min-w-0 flex-1 font-mono text-sm"
        />
        <Button data-testid="fleet-label-add" size="sm" variant="outline" onClick={add}>
          Add
        </Button>
      </span>
      <span className="flex items-center gap-2">
        <Button
          data-testid="fleet-labels-save"
          size="sm"
          variant="outline"
          onClick={() => send({ type: 'SetFleetMemberLabels', data: { context, labels } })}
        >
          Save labels
        </Button>
        <Button
          data-testid="fleet-member-forget"
          size="sm"
          variant="ghost"
          onClick={() => send({ type: 'ForgetFleetMember', data: { context } })}
        >
          Forget this member
        </Button>
      </span>
      {message(labelsRejected)}
    </div>
  )
}
