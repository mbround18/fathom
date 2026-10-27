/**
 * The grant editor (spec 055 US6 T038): a team's role on a workspace, optionally scoped to some
 * host profiles.
 *
 * Profiles are a multi-select: the profiles other grants already name are offered as checkboxes,
 * and any other profile can be typed in. None chosen means the whole workspace. The preview that
 * follows lists every user the grant reaches.
 */
import { XIcon } from '@phosphor-icons/react'
import { Button, Checkbox, Input } from '@companion/ui'
import { useId, useState } from 'react'

import type { TeamInfo, TeamRole } from '@/protocol/wire'

import { MAX_PROFILES, PROFILE_PATTERN, ROLE_LABEL, TEAM_ROLES } from './labels'
import { Field, InlineForm, NativeSelect } from './parts'
import { useGovernance } from './store'

function ProfilePicker({
  known,
  chosen,
  onChange,
}: {
  known: readonly string[]
  chosen: readonly string[]
  onChange: (next: string[]) => void
}) {
  const [typed, setTyped] = useState('')
  const typedId = useId()
  const groupId = useId()
  const options = [...new Set([...known, ...chosen])].sort()
  const toggle = (p: string, on: boolean) => onChange(on ? [...chosen, p] : chosen.filter((x) => x !== p))
  const candidate = typed.trim()
  const canAdd = PROFILE_PATTERN.test(candidate) && !chosen.includes(candidate) && chosen.length < MAX_PROFILES
  const add = () => {
    if (!canAdd) return
    onChange([...chosen, candidate])
    setTyped('')
  }
  return (
    <fieldset aria-labelledby={groupId} className="flex min-w-0 basis-full flex-col gap-1">
      <legend id={groupId} className="text-xs text-muted-foreground">
        Profiles <span className="text-muted-foreground/80">(optional; none = the whole workspace)</span>
      </legend>
      {options.length > 0 && (
        <ul className="flex min-w-0 flex-wrap gap-x-3 gap-y-1">
          {options.map((p) => (
            <li key={p} className="flex items-center gap-1">
              <Checkbox
                id={`${groupId}-${p}`}
                checked={chosen.includes(p)}
                onCheckedChange={(v) => toggle(p, v === true)}
              />
              <label htmlFor={`${groupId}-${p}`} className="font-mono text-xs">
                {p}
              </label>
            </li>
          ))}
        </ul>
      )}
      <div className="flex min-w-0 items-center gap-1">
        <label htmlFor={typedId} className="sr-only">
          Another profile
        </label>
        <Input
          id={typedId}
          value={typed}
          placeholder="Another profile"
          autoComplete="off"
          maxLength={63}
          className="h-7 min-w-0 flex-1 basis-36 text-xs"
          aria-invalid={candidate !== '' && !PROFILE_PATTERN.test(candidate)}
          onChange={(e) => setTyped(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter') {
              e.preventDefault()
              add()
            }
          }}
        />
        <Button type="button" variant="outline" size="xs" disabled={!canAdd} onClick={add}>
          Add profile
        </Button>
      </div>
      {chosen.length > 0 && (
        <p data-chosen-profiles className="flex min-w-0 flex-wrap items-center gap-1 text-xs">
          {chosen.map((p) => (
            <span key={p} className="inline-flex items-center gap-0.5 border border-border px-1 font-mono">
              {p}
              <button type="button" aria-label={`Drop ${p}`} onClick={() => toggle(p, false)} className="text-muted-foreground hover:text-foreground">
                <XIcon aria-hidden className="size-3" />
              </button>
            </span>
          ))}
        </p>
      )}
    </fieldset>
  )
}

export function GrantEditor({ team, knownProfiles }: { team: TeamInfo; knownProfiles: readonly string[] }) {
  const propose = useGovernance((s) => s.propose)
  const workspaces = useGovernance((s) => s.workspaces)
  const [workspace, setWorkspace] = useState('')
  const [role, setRole] = useState<TeamRole>('viewer')
  const [profiles, setProfiles] = useState<string[]>([])
  const wsId = useId()
  const roleId = useId()
  const usable = (workspaces ?? []).filter((w) => !w.archived)
  return (
    <InlineForm
      label={`Add a grant to ${team.name}`}
      onSubmit={() => {
        if (!workspace) return
        propose('Add grant', { type: 'PreviewGrantAdd', data: { team: team.id, workspace, role, profiles } })
        setProfiles([])
      }}
    >
      <Field label="Workspace" htmlFor={wsId}>
        <NativeSelect id={wsId} value={workspace} onChange={(e) => setWorkspace(e.target.value)}>
          <option value="">Choose a workspace…</option>
          {usable.map((w) => (
            <option key={w.id} value={w.id}>
              {w.slug}
            </option>
          ))}
        </NativeSelect>
      </Field>
      <Field label="Role" htmlFor={roleId}>
        <NativeSelect id={roleId} value={role} onChange={(e) => setRole(e.target.value as TeamRole)}>
          {TEAM_ROLES.map((r) => (
            <option key={r} value={r}>
              {ROLE_LABEL[r]}
            </option>
          ))}
        </NativeSelect>
      </Field>
      <ProfilePicker known={knownProfiles} chosen={profiles} onChange={setProfiles} />
      <Button type="submit" size="sm" disabled={!workspace}>
        Add grant
      </Button>
    </InlineForm>
  )
}
