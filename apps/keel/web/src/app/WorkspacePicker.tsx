/**
 * What a workspace-scoped page draws in its header (spec 060 T053, spec 066 T032). The choice of
 * workspace is the shell's switcher now; a page only loads its lists for the current workspace.
 * A person with no workspace sees why the page is empty and who can fix it: an owner or admin is
 * sent to create one, anyone else is told who grants them (spec 063 FR-013).
 */
import { Button } from '@companion/ui'
import { EmptyState, Link } from '@companion/ui/kit'
import { useEffect } from 'react'

import { canSeeSettings } from '@/features/settings/tabs'
import { useGovernance } from '@/features/teams/store'

import { useSession } from './session-store'
import { useCurrentWorkspace } from './workspace-store'

/** No workspace yet: the guided first step for an owner or admin, who grants them for anyone else. */
export function NoWorkspace() {
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

/**
 * Loads the page's lists for the current workspace through `load`, again whenever it changes.
 * Draws nothing, or the guided empty state once the list says there is no workspace.
 */
export function CurrentWorkspace({ load }: { load: (workspace: string) => void }) {
  const current = useCurrentWorkspace()
  const listed = useGovernance((s) => s.workspaces !== null)
  const id = current?.id ?? null

  useEffect(() => {
    if (id) load(id)
  }, [id, load])

  return listed && id === null ? <NoWorkspace /> : null
}
