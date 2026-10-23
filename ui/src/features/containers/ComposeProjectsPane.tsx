/**
 * 048-docker-management US1: the compose projects this computer has, one row each.
 *
 * The Containers scene groups containers by project; this pane answers the other half of the same
 * question — *which projects are on this machine, what declares each one, and how much of each is
 * running*. The strays bucket is not a project and so is not listed here: it has no definition file
 * and no name of its own, and a row for it would be a row about nothing (FR-005).
 *
 * The verbs are the project's own, which is why they are the same component the scene mounts
 * rather than a second set written beside it: `up` and `down` need the project's file, the grant
 * that allows them is the same grant, and a preview taken here must look exactly like one taken
 * there (FR-029).
 */
import { useState } from 'react'

import { Button } from '@companion/ui'
import { useStore } from '@/store'

import { definitionLabel, effectiveAdoption, groupKey } from './labels'
import { ProjectActions } from './ProjectActions'
import { UsagePanel } from './UsagePanel'

/** What the grant in force right now reads as. */
function grantWord(state: ReturnType<typeof effectiveAdoption>): string {
  if (state === 'adopted') return 'Adopted'
  if (state === 'not_adopted') return 'Read-only'
  return `Adopted with ${state.implicit_via_managed_cluster}`
}

export function ComposeProjectsPane() {
  const inventory = useStore((s) => s.containerRuntime.inventory)
  const granted = useStore((s) => s.containerRuntime.adopted)
  const refresh = useStore((s) => s.refreshContainerInventory)
  const dismiss = useStore((s) => s.dismissContainerOperation)
  const [selected, setSelected] = useState<string | null>(null)

  // The presence states — absent, stopped, refusing — are told once, by the Containers scene the
  // nav lands on first. This pane is the same reading of the same daemon, so while that reading
  // has not arrived there is nothing here to say about it.
  if (!inventory) return null

  const projects = inventory.projects.filter((p) => p.group.kind === 'project')
  const chosen = projects.find((p) => groupKey(p) === selected)

  return (
    <div
      data-testid="compose-projects-pane"
      className="flex h-full min-h-0 flex-col overflow-hidden"
    >
      <div className="flex min-w-0 shrink-0 items-center gap-2 border-b border-border px-3 py-2">
        <p className="min-w-0 flex-1 truncate text-xs text-muted-foreground">
          {projects.length === 0
            ? 'No compose project is running on this computer.'
            : `${String(projects.length)} compose ${projects.length === 1 ? 'project' : 'projects'}`}
          {' · read '}
          <time dateTime={inventory.as_of}>
            {new Date(inventory.as_of).toLocaleTimeString([], {
              hour: '2-digit',
              minute: '2-digit',
              second: '2-digit',
            })}
          </time>
        </p>
        <Button
          size="sm"
          variant="ghost"
          data-testid="compose-projects-refresh"
          onClick={() => {
            refresh(['container'])
          }}
        >
          Refresh
        </Button>
      </div>

      <ul className="min-h-24 min-w-0 flex-1 overflow-y-auto">
        {projects.map((project) => {
          const key = groupKey(project)
          const file = definitionLabel(project)
          const live = effectiveAdoption(project, granted)
          return (
            <li key={key} className="min-w-0 border-b border-border/60">
              <button
                type="button"
                data-testid={`compose-project-${key}`}
                aria-pressed={key === selected}
                className="flex w-full min-w-0 flex-col items-start gap-0.5 px-3 py-2 text-left hover:bg-muted/60 aria-pressed:bg-muted"
                onClick={() => {
                  setSelected((current) => (current === key ? null : key))
                  dismiss()
                }}
              >
                <span className="flex min-w-0 items-baseline gap-2">
                  <span className="min-w-0 truncate text-sm">{key}</span>
                  <span className="shrink-0 text-xs text-muted-foreground">
                    {project.running} of {project.members} running · {grantWord(live)}
                  </span>
                </span>
                {/* A project with no file on disk says so by having no file line: the verbs that
                    would need one are not offered either, and two ways of saying the same absence
                    is one too many. */}
                {file !== null && (
                  <span
                    data-testid={`compose-project-file-${key}`}
                    className="min-w-0 truncate font-mono text-[11px] text-muted-foreground"
                  >
                    {file}
                  </span>
                )}
              </button>
            </li>
          )
        })}
      </ul>

      {chosen !== undefined && (
        <div className="flex max-h-[60%] min-w-0 shrink-0 flex-col overflow-y-auto">
          <ProjectActions project={chosen} />
          <UsagePanel key={groupKey(chosen)} containers={chosen.containers} />
        </div>
      )}
    </div>
  )
}
