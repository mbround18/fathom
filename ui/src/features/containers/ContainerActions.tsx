/**
 * 048-docker-management US2: the verbs for one container.
 *
 * The same two-phase path the project verbs use, pointed at a single id — one preview, one
 * confirmation, and a row that changes only when the runtime says so (FR-021). The scope is
 * `{kind:'container'}` rather than the project, so the preview names this container and nothing
 * else: a mis-click here cannot reach a sibling.
 *
 * Which verbs are offered follows the container's state, because a Start on a running container
 * and a Stop on an exited one are both commands whose only possible outcome is an error the
 * operator did not need to see. Kill stays available whenever the container is up: it is the verb
 * for a container that is ignoring Stop, which is the case this feature was asked for.
 */
import { Button } from '@companion/ui'
import type { Container, LifecycleAction } from '@/protocol/types'
import { useStore } from '@/store'

/** The verbs this container's state makes sense of, in the order they are offered. */
function verbsFor(container: Container): LifecycleAction[] {
  const up = container.state === 'running' || container.state === 'restarting'
  if (container.state === 'paused') return ['unpause', 'restart', 'kill']
  if (!up) return ['start', 'restart']
  return ['stop', 'restart', 'pause', 'kill']
}

const LABELS: Record<LifecycleAction, string> = {
  start: 'Start',
  stop: 'Stop',
  restart: 'Restart',
  pause: 'Pause',
  unpause: 'Unpause',
  kill: 'Kill',
}

export function ContainerActions({ container }: { container: Container }) {
  const preview = useStore((s) => s.previewContainerOperation)
  const operation = useStore((s) => s.containerRuntime.operation)
  // A second preview while one is in flight would leave two argvs on screen and one confirmation
  // field between them (FR-029).
  const busy = operation !== undefined

  return (
    <div
      data-testid={`container-actions-${container.name}`}
      className="flex min-w-0 flex-col gap-2"
    >
      <p className="font-mono text-xs text-muted-foreground">{container.name}</p>
      <div className="flex min-w-0 flex-wrap gap-2">
        {verbsFor(container).map((action) => (
          <Button
            key={action}
            size="sm"
            variant={action === 'kill' ? 'destructive' : action === 'stop' ? 'default' : 'ghost'}
            data-testid={`container-${action}-${container.name}`}
            disabled={busy}
            onClick={() => {
              preview({
                kind: 'lifecycle',
                action,
                scope: { kind: 'container', id: container.id },
              })
            }}
          >
            {LABELS[action]}
          </Button>
        ))}
      </div>
    </div>
  )
}
