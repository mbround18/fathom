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
import { Button } from '@/components/ui/button'
import type { Project } from '@/protocol/types'
import { useStore } from '@/store'

import { AdoptionControl } from './AdoptionControl'
import { groupKey } from './labels'
import { OperationPreview } from './OperationPreview'

export function ProjectActions({ project }: { project: Project }) {
  const preview = useStore((s) => s.previewContainerOperation)
  const operation = useStore((s) => s.containerRuntime.operation)

  const key = groupKey(project)
  const named = project.group.kind === 'project' ? project.group.name : null
  const adopted = project.adoption !== 'not_adopted'
  const hasFile = project.definition_state === 'present'
  // A second preview while one is in flight would leave two argvs on screen and one confirmation
  // field between them (FR-029).
  const busy = operation !== undefined

  return (
    <div
      data-testid={`project-actions-${key}`}
      className="flex shrink-0 flex-col gap-2 border-t border-border bg-background/60 p-3"
    >
      <AdoptionControl project={project} />

      {named !== null && adopted && (
        <div className="flex min-w-0 flex-wrap gap-2">
          <Button
            size="sm"
            data-testid={`project-stop-${key}`}
            disabled={busy}
            onClick={() => {
              preview({ kind: 'lifecycle', action: 'stop', scope: { kind: 'project', project: named } })
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
