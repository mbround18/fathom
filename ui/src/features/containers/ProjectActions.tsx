/**
 * 048-docker-management US2 (T047): the project-scoped verbs, for one project at a time.
 *
 * One preview, one confirmation, per-member outcomes (FR-029). That is why the verbs here are
 * project-scoped rather than a row-by-row fan-out: `docker stop a b c` is one command naming every
 * member, so there is one argv to show and one thing to approve, and the per-member results come
 * back attributed from the runtime's own output. Four confirmations for one intention would be
 * four chances to approve something other than what was read.
 *
 * `up` and `down` go through compose and need the project's own file; `stop` and `restart` act on
 * the containers directly, so they work whether or not the project declares one. The verbs that
 * need a file are not offered when there is none — and the file line in the heading already says
 * why, rather than a button that exists to refuse.
 */
import { Button } from '@companion/ui'
import type { Container, Project } from '@/protocol/types'
import { useStore } from '@/store'

import { AdoptionControl } from './AdoptionControl'
import { ContainerActions } from './ContainerActions'
import { definitionLabel, containerIsActionable, effectiveAdoption, groupKey } from './labels'
import { OperationPreview } from './OperationPreview'

export function ProjectActions({
  project,
  container,
}: {
  project: Project
  /** When one container is selected, its verbs replace the project's — same grant, one object. */
  container?: Container
}) {
  const preview = useStore((s) => s.previewContainerOperation)
  const operation = useStore((s) => s.containerRuntime.operation)
  const granted = useStore((s) => s.containerRuntime.adopted)

  const key = groupKey(project)
  const named = project.group.kind === 'project' ? project.group.name : null
  // The live grant, not the inventory's snapshot of it: a project adopted a moment ago is
  // adopted now (FR-015).
  const adopted = effectiveAdoption(project, granted) !== 'not_adopted'
  const hasFile = project.definition_state === 'present'
  // A second preview while one is in flight would leave two argvs on screen and one confirmation
  // field between them (FR-029).
  const busy = operation !== undefined

  return (
    <div
      data-testid={`project-actions-${key}`}
      className="flex shrink-0 flex-col gap-2 border-t border-border bg-background/60 p-3"
    >
      <AdoptionControl project={project} container={container} />

      {/* The container's own rights, not the group's: a kind node has no compose project to
          adopt, and its bucket must stay read-only for every stray beside it (FR-016). */}
      {container !== undefined && containerIsActionable(container, granted) && (
        <ContainerActions container={container} />
      )}

      {named !== null && adopted && container === undefined && (
        <div className="flex min-w-0 flex-wrap gap-2">
          <Button
            size="sm"
            data-testid={`project-stop-${key}`}
            disabled={busy}
            onClick={() => {
              preview({
                kind: 'lifecycle',
                action: 'stop',
                scope: { kind: 'project', project: named },
              })
            }}
          >
            Stop all {project.members}
          </Button>
          <Button
            size="sm"
            variant="ghost"
            data-testid={`project-restart-${key}`}
            disabled={busy}
            onClick={() => {
              preview({
                kind: 'lifecycle',
                action: 'restart',
                scope: { kind: 'project', project: named },
              })
            }}
          >
            Restart all
          </Button>
          {/* FR-033: a project whose file Fathom could not read has no project-level operations,
              and the reason is on screen rather than two buttons quietly missing. The last
              sentence is the one that matters most: the operator's file is still theirs. */}
          {!hasFile && (
            <p
              data-testid={`project-no-definition-${key}`}
              className="min-w-0 flex-1 text-xs text-muted-foreground"
            >
              {definitionLabel(project) ?? 'This project declares no definition file'}. compose up,
              down and restart are unavailable for it; Fathom has not changed the file. Its
              containers can still be stopped and restarted individually.
            </p>
          )}
          {hasFile && (
            <>
              <Button
                size="sm"
                variant="ghost"
                data-testid={`project-up-${key}`}
                disabled={busy}
                onClick={() => {
                  preview({ kind: 'project_lifecycle', project: named, action: 'up' })
                }}
              >
                compose up
              </Button>
              <Button
                size="sm"
                variant="destructive"
                data-testid={`project-down-${key}`}
                disabled={busy}
                onClick={() => {
                  preview({ kind: 'project_lifecycle', project: named, action: 'down' })
                }}
              >
                compose down
              </Button>
            </>
          )}
        </div>
      )}

      <OperationPreview />
    </div>
  )
}
