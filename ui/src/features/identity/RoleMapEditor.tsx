/** Edit the three Fathom-level role group names (OidcSetRoleMap). Admin only; never escalates. */
import { useState } from 'react'

import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import type { FathomRoleMap } from '@/protocol/types'
import { useStore } from '@/store'

import { isSignedIn } from './state'

const ROWS: { key: keyof FathomRoleMap; label: string }[] = [
  { key: 'viewers_group', label: 'Viewer group' },
  { key: 'editors_group', label: 'Editor group' },
  { key: 'admins_group', label: 'Admin group' },
]

const blank = (m: Partial<FathomRoleMap>): FathomRoleMap => ({
  viewers_group: '',
  editors_group: '',
  admins_group: '',
  ...m,
})

export function RoleMapEditor() {
  const send = useStore((s) => s.send)
  const saved = useStore((s) => s.identity.config.role_map)
  const identities = useStore((s) => s.identity.identities)
  const me = Object.values(identities).find(isSignedIn)
  // Editable only by a signed-in admin, or while no admin group is configured (bootstrap).
  const isAdmin = !saved.admins_group || (!!me && me.groups.includes(saved.admins_group))
  const [edits, setEdits] = useState<Partial<FathomRoleMap>>({})
  const draft: FathomRoleMap = { ...blank(saved), ...edits }
  const dirty = ROWS.some((r) => draft[r.key] !== blank(saved)[r.key])

  return (
    <fieldset className="flex flex-col gap-2" data-testid="oidc-role-map" disabled={!isAdmin}>
      <legend className="text-xs font-semibold uppercase">Role groups</legend>
      <div className="grid grid-cols-1 gap-2 sm:grid-cols-3">
        {ROWS.map((r) => (
          <label key={r.key} className="flex flex-col gap-1 text-xs">
            <span className="text-muted-foreground">{r.label}</span>
            <Input
              data-testid={`oidc-role-${r.key}`}
              value={draft[r.key]}
              onChange={(e) => setEdits((d) => ({ ...d, [r.key]: e.target.value }))}
            />
          </label>
        ))}
      </div>
      {!isAdmin && (
        <p className="text-xs text-muted-foreground" data-testid="oidc-role-map-readonly">
          Only administrators can change role groups.
        </p>
      )}
      <div>
        <Button
          size="sm"
          disabled={!isAdmin || !dirty}
          data-testid="oidc-role-map-save"
          onClick={() => {
            setEdits({})
            send({
              type: 'OidcSetRoleMap',
              data: {
                role_map: {
                  viewers_group: draft.viewers_group.trim(),
                  editors_group: draft.editors_group.trim(),
                  admins_group: draft.admins_group.trim(),
                },
              },
            })
          }}
        >
          Save role groups
        </Button>
      </div>
    </fieldset>
  )
}
