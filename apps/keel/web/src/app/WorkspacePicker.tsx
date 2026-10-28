/**
 * The workspace picker in a page header (spec 060 T053), one look for every section. A person
 * with no workspace sees why the page is empty and who can fix it.
 */
import { useEffect, useId } from 'react'

import { LABEL, Select } from '@/ui/form'

import { useSession } from './session-store'

export interface WorkspaceOption {
  id: string
  label: string
}

/** The label and select; with no workspaces, the empty line instead. */
export function WorkspaceSelect({
  options,
  value,
  onChange,
  empty,
}: {
  options: readonly WorkspaceOption[]
  value: string
  onChange: (id: string) => void
  empty: string
}) {
  const id = useId()
  if (options.length === 0)
    return (
      <p data-pane-state="empty" className="text-xs text-muted-foreground">
        {empty}
      </p>
    )
  return (
    <div className="flex min-w-0 items-center gap-2">
      <label htmlFor={id} className={LABEL}>
        Workspace
      </label>
      <Select
        id={id}
        className="max-w-full"
        value={value}
        onChange={(e) => {
          onChange(e.target.value)
        }}
      >
        {options.map((o) => (
          <option key={o.id} value={o.id}>
            {o.label}
          </option>
        ))}
      </Select>
    </div>
  )
}

/**
 * The workspaces the signed-in person holds a grant in. Loads the chosen one (the first, until
 * the person picks another) through `load`.
 */
export function GrantWorkspacePicker({
  workspace,
  load,
}: {
  workspace: string | null
  load: (workspace: string) => void
}) {
  const grants = useSession((s) => s.me?.grants)
  const first = grants?.[0]?.workspace ?? null
  const options = [...new Map((grants ?? []).map((g) => [g.workspace, g.workspace_slug])).entries()].map(
    ([id, label]) => ({ id, label }),
  )

  useEffect(() => {
    const current = workspace ?? first
    if (current) load(current)
  }, [workspace, first, load])

  return (
    <WorkspaceSelect
      options={options}
      value={workspace ?? first ?? ''}
      onChange={load}
      empty="You have no workspace yet. An owner or admin grants one from Teams."
    />
  )
}
