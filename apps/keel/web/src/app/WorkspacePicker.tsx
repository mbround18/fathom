/**
 * The workspace picker in a page header (spec 060 T053), one look for every section. A person
 * with no workspace sees why the page is empty and who can fix it: an owner or admin is sent to
 * Teams to create one, anyone else is told who grants them (spec 063 FR-013).
 */
import { Button } from '@companion/ui'
import { Link } from '@companion/ui/kit'
import { useEffect, useId } from 'react'

import { canSeeSettings } from '@/features/settings/tabs'
import { TermTip } from '@/help/TermTip'
import { EmptyState } from '@/ui/EmptyState'
import { LABEL, Select } from '@/ui/form'

import { useSession } from './session-store'

export interface WorkspaceOption {
  id: string
  label: string
}

/** No workspace yet: the guided first step for an owner or admin, who grants them for anyone else. */
function NoWorkspace() {
  const role = useSession((s) => s.me?.instance_role)
  const owner = canSeeSettings(role)
  return (
    <div className="basis-full">
      <EmptyState
        page="workspace-picker"
        title={owner ? 'Create a workspace' : 'No workspace yet'}
        reason="Hosts, bindings and runs all live in a workspace."
        steps={owner ? ['Create a workspace.', 'Grant a team access to it.', 'Enroll a host into it.'] : undefined}
        action={
          owner ? (
            <Button asChild size="sm">
              <Link to="/teams">Create a workspace</Link>
            </Button>
          ) : undefined
        }
        whoCan="An owner or admin"
        verb="grant you a workspace from Teams"
        guide={{ guide: 'getting-started', anchor: 'create-a-workspace' }}
      />
    </div>
  )
}

/** The label and select; with no workspaces, the guided empty state instead. */
export function WorkspaceSelect({
  options,
  value,
  onChange,
}: {
  options: readonly WorkspaceOption[]
  value: string
  onChange: (id: string) => void
}) {
  const id = useId()
  if (options.length === 0) return <NoWorkspace />
  return (
    <div className="flex min-w-0 items-center gap-2">
      <span className="inline-flex shrink-0 items-center gap-1">
        <label htmlFor={id} className={LABEL}>
          Workspace
        </label>
        <TermTip term="workspace" />
      </span>
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
    />
  )
}
