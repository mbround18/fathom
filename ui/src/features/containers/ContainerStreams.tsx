/**
 * 048-docker-management US5 (T067): one container's own words, and a shell in one that was adopted.
 *
 * Two panes over the same selected container, and neither of them opens on its own. A log follow is
 * a process on this machine for as long as the pane is on screen, so the operator asks for it
 * (Principle V) — and because nothing starts until asked, moving the selection or leaving the scene
 * leaves nothing streaming, which is the same sentence as FR-036's "closing the view ends it leaving
 * no process behind".
 *
 * Logs are offered for every container, adopted or not (FR-035): reading a container's output
 * changes nothing about it. A shell is offered only where a mutation would be allowed (FR-013), and
 * where it is not, the reason is on screen instead of a button that exists to refuse — the adoption
 * control that would grant it is already directly above this, in `ProjectActions`.
 */
import { TerminalIcon, TextAlignLeftIcon, XIcon } from '@phosphor-icons/react'
import { useCallback, useEffect, useRef, useState } from 'react'

import { LogScroller } from '@/components/panels/LogScroller'
import { type TerminalTransport, TerminalSurface } from '@/components/panels/TerminalView'
import { Button } from '@/components/ui/button'
import type { Container } from '@/protocol/types'
import { useStore } from '@/store'

import { containerIsActionable } from './labels'

type Pane = 'none' | 'logs' | 'shell'

export function ContainerStreams({ container }: { container: Container }) {
  const granted = useStore((s) => s.containerRuntime.adopted)
  const [pane, setPane] = useState<Pane>('none')

  // Nothing resets `pane` when the selection moves, because nothing has to: the scene keys this
  // component on the container's id, so a different container is a different component with its
  // own closed panes — and the old one unmounts, which is what ends its streams.

  const mayShell = containerIsActionable(container, granted)

  return (
    <div
      data-testid={`container-streams-${container.id}`}
      data-pane={pane}
      className="flex shrink-0 flex-col border-t border-border"
    >
      <div className="flex min-w-0 shrink-0 items-center gap-2 px-3 py-1.5">
        <Button
          size="xs"
          variant={pane === 'logs' ? 'default' : 'ghost'}
          data-testid={`container-logs-open-${container.id}`}
          onClick={() => {
            setPane((p) => (p === 'logs' ? 'none' : 'logs'))
          }}
        >
          <TextAlignLeftIcon data-icon="inline-start" /> Logs
        </Button>
        {mayShell ? (
          <Button
            size="xs"
            variant={pane === 'shell' ? 'default' : 'ghost'}
            data-testid={`container-shell-open-${container.id}`}
            onClick={() => {
              setPane((p) => (p === 'shell' ? 'none' : 'shell'))
            }}
          >
            <TerminalIcon data-icon="inline-start" /> Shell
          </Button>
        ) : (
          <p
            data-testid={`container-shell-denied-${container.id}`}
            className="min-w-0 flex-1 truncate text-xs text-muted-foreground"
          >
            Adopt this project to open a shell in it. Its logs are readable either way.
          </p>
        )}
        {pane !== 'none' && (
          <Button
            size="xs"
            variant="ghost"
            className="ml-auto"
            data-testid={`container-stream-close-${container.id}`}
            onClick={() => {
              setPane('none')
            }}
          >
            <XIcon data-icon="inline-start" /> Close
          </Button>
        )}
      </div>

      {/* Height is the pane's, not the content's: a terminal needs a box to fit itself into, and a
          log follow that grew the scene as it arrived would move everything above it. */}
      {pane === 'logs' && <ContainerLogsPane container={container} />}
      {pane === 'shell' && <ContainerShellPane container={container} />}
    </div>
  )
}

function ContainerLogsPane({ container }: { container: Container }) {
  const start = useStore((s) => s.startContainerLogs)
  const stop = useStore((s) => s.stopContainerLogs)
  // The follow's id lives in a ref rather than state: the store update that creates the entry is
  // what re-renders this pane, so the id does not need to.
  const streamIdRef = useRef(0)
  const [attempt, setAttempt] = useState(0)
  const stream = useStore((s) => {
    const entry = s.streams[streamIdRef.current]
    return entry?.kind === 'container-logs' ? entry : undefined
  })

  // One follow per mount, ended on unmount. `owner` is the project the container fell into, so a
  // backend refusal can name it.
  useEffect(() => {
    const id = start(container.id, container.project ?? undefined)
    streamIdRef.current = id
    return () => {
      stop(id)
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps -- `attempt` is the restart button
  }, [container.id, container.project, start, stop, attempt])

  const restart = useCallback(() => {
    setAttempt((a) => a + 1)
  }, [])

  const lines = stream?.lines.toArray() ?? []

  return (
    <div className="flex h-64 min-h-0 flex-col">
      <LogScroller
        lines={lines}
        truncated={stream?.lines.truncated ?? 0}
        rev={stream?.rev ?? 0}
        status={stream?.status ?? 'starting'}
        error={stream?.error}
        reason={stream?.reason}
        startingLabel={`Following ${container.name}`}
        errorTitle="Could not follow this container"
        onRestart={restart}
        testId={`container-logs-scroll-${container.id}`}
      />
    </div>
  )
}

function ContainerShellPane({ container }: { container: Container }) {
  const send = useStore((s) => s.send)
  const start = useStore((s) => s.startContainerSession)
  const stop = useStore((s) => s.stopContainerSession)
  // The client mints the session's id, so this pane knows its own stream by number and needs none
  // of the pod terminal's "newest entry since" guesswork.
  const streamIdRef = useRef(0)

  const transport: TerminalTransport = {
    key: container.id,
    start: () => {
      streamIdRef.current = start(container.id)
    },
    stop: () => {
      if (streamIdRef.current !== 0) stop(streamIdRef.current)
      streamIdRef.current = 0
    },
    input: (data) => {
      send({
        type: 'SendContainerSessionInput',
        data: { stream_id: streamIdRef.current, data },
      })
    },
    resize: (cols, rows) => {
      if (streamIdRef.current === 0) return
      send({
        type: 'ResizeContainerSession',
        data: { stream_id: streamIdRef.current, cols, rows },
      })
    },
    select: (streams) => {
      const entry = streams[streamIdRef.current]
      return entry?.kind === 'container-session' ? entry : undefined
    },
  }

  return (
    <div className="flex h-64 min-h-0 flex-col">
      <TerminalSurface transport={transport} testId={`container-shell-${container.id}`} />
    </div>
  )
}
