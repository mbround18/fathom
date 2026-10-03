/**
 * 048-docker-management US1: the Containers scene.
 *
 * Four presence states, rendered four different ways, because the one thing this scene must never
 * do is answer "no containers" to a question nobody asked (FR-003, FR-004). A runtime that is not
 * installed, one that is installed and stopped, one that refuses the socket, and one that is
 * working and has nothing running are four different facts, and only the last of them is an empty
 * list.
 *
 * The inventory is read once when the scene opens and then only when the operator asks. Nothing
 * polls: a scene nobody is looking at costs nothing, and a scene being looked at does not quietly
 * run `docker ps` every few seconds behind an operator's back (Principle V). The cost of that is
 * that a list can go out of date, which is why the time it was read is on screen and why a runtime
 * that goes away marks the list stale instead of blanking it — the last true reading is more use
 * than an empty one.
 */
import { ArrowsClockwiseIcon, LockSimpleIcon, PlugsIcon, WarningIcon } from '@phosphor-icons/react'
import { useState } from 'react'

import { Loading, StateFrame } from '@/components/shell/PaneStates'
import { Button } from '@/components/ui/button'
import type { KindRuntimeKind } from '@/protocol/types'
import { useStore } from '@/store'

import { ContainerGroupList } from './ContainerGroupList'
import { groupKey, type Selection } from './labels'
import { ProjectActions } from './ProjectActions'

/** The runtime's name as a person writes it. */
function runtimeName(runtime: KindRuntimeKind | null): string {
  if (runtime === 'docker') return 'Docker'
  if (runtime === 'podman') return 'Podman'
  return 'The container runtime'
}

/**
 * What to do about a refusal, when the refusal is one an operator can act on.
 *
 * Only the permission case earns a remediation, and it is matched on the runtime's own words
 * rather than invented: a daemon that refuses for some other reason gets its sentence shown and
 * no advice, because advice that does not fit the failure is worse than none.
 */
function remediationFor(detail: string, endpoint: string | null): string | null {
  if (!/permission denied/i.test(detail)) return null
  const socket = endpoint ?? 'the runtime socket'
  return `Your user cannot read ${socket}. Add it to the group that owns the socket (on Docker, \`sudo usermod -aG docker $USER\`) and sign in again, or run the runtime rootless. Fathom will not change the socket's permissions for you.`
}

function Unavailable({ detail, endpoint }: { detail: string; endpoint: string | null }) {
  const remediation = remediationFor(detail, endpoint)
  return (
    <StateFrame
      kind="runtime-unreachable"
      tone="err"
      icon={LockSimpleIcon}
      title="The container runtime refused"
      detail={
        <>
          {/* Verbatim: the runtime's own sentence names the socket and says what went wrong far
              better than a rewording of it would. */}
          <span data-testid="containers-runtime-detail" className="block font-mono">
            {detail}
          </span>
          {remediation && (
            <span data-testid="containers-remediation" className="mt-2 block">
              {remediation}
            </span>
          )}
        </>
      }
    />
  )
}

function sameSelection(a: Selection | null, b: Selection): boolean {
  if (a === null || a.kind !== b.kind) return false
  return a.kind === 'project' && b.kind === 'project'
    ? a.key === b.key
    : a.kind === 'container' && b.kind === 'container' && a.id === b.id
}

export function ContainersScene() {
  const presence = useStore((s) => s.containerRuntime.presence)
  const probed = useStore((s) => s.containerRuntime.probed)
  const inventory = useStore((s) => s.containerRuntime.inventory)
  const stale = useStore((s) => s.containerRuntime.stale)
  const refresh = useStore((s) => s.refreshContainerInventory)
  const dismiss = useStore((s) => s.dismissContainerOperation)
  const [selected, setSelected] = useState<Selection | null>(null)

  // No open-read here: navigating to a section already sends its `refreshCmd`, which for this
  // one is `RefreshContainerInventory { classes: ['container'] }`. A second read from the scene
  // would be one listing per open too many, and against a runtime that answers once and then
  // refuses it would report the runtime as having gone away the moment the scene appeared.

  if (!probed) return <Loading label="Looking for a container runtime" />

  if (presence === undefined || presence.state === 'absent') {
    return (
      <StateFrame
        kind="runtime-absent"
        icon={PlugsIcon}
        title="No container runtime on this computer"
        detail="Fathom found neither Docker nor Podman. Install one and this scene appears on its own — nothing here needs enabling."
      />
    )
  }

  if (presence.state === 'stopped') {
    return (
      <StateFrame
        kind="runtime-stopped"
        tone="warn"
        icon={WarningIcon}
        title={`${runtimeName(presence.runtime)} is installed and not running`}
        detail="Start it and refresh. This is not the same as having no containers: Fathom cannot see any until the runtime answers."
      />
    )
  }

  if (presence.state === 'unreachable') {
    return (
      <Unavailable
        detail={presence.detail ?? 'The runtime did not answer.'}
        endpoint={presence.endpoint}
      />
    )
  }

  // Ready, and the first read has not come back yet.
  if (!inventory) {
    if (stale !== undefined) return <Unavailable detail={stale} endpoint={presence.endpoint} />
    return <Loading label="Reading the container inventory" />
  }

  return (
    <div
      data-testid="containers-scene"
      data-stale={stale === undefined ? 'false' : 'true'}
      className="flex h-full min-h-0 flex-col overflow-hidden"
    >
      <div className="flex min-w-0 shrink-0 items-center gap-2 border-b border-border px-3 py-2">
        <p className="min-w-0 flex-1 truncate text-xs text-muted-foreground">
          {inventory.projects.length === 0
            ? `${runtimeName(presence.runtime)} is running and has no containers`
            : `${String(inventory.projects.length)} groups`}
          {' · read '}
          <time dateTime={inventory.as_of} data-testid="containers-as-of">
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
          data-testid="containers-refresh"
          onClick={() => {
            refresh(['container'])
          }}
        >
          <ArrowsClockwiseIcon aria-hidden="true" className="size-3.5" />
          Refresh
        </Button>
      </div>

      {stale !== undefined && (
        /* The list stays. A reading from a minute ago with the reason it stopped being updated
           beside it is more use to an operator than an empty screen (FR-003). */
        <p
          role="status"
          data-testid="containers-stale"
          className="shrink-0 border-b border-status-warn/50 bg-status-warn/10 px-3 py-1.5 text-xs text-status-warn"
        >
          This list is from{' '}
          <time dateTime={inventory.as_of}>
            {new Date(inventory.as_of).toLocaleTimeString([], {
              hour: '2-digit',
              minute: '2-digit',
              second: '2-digit',
            })}
          </time>{' '}
          and is no longer being updated: {stale}
        </p>
      )}

      <ContainerGroupList
        projects={inventory.projects}
        selected={selected}
        onSelect={(next) => {
          // Changing selection drops whatever preview was open: a preview belongs to the operation
          // it was taken for, and one left on screen beside another object's verbs is the kind of
          // mismatch FR-018 exists to prevent.
          setSelected((current) => (sameSelection(current, next) ? null : next))
          dismiss()
        }}
      />

      {/* The selected object's verbs live outside the virtualized list: a row whose height grows
          when it is chosen would move every row below it. */}
      {(() => {
        if (selected === null) return null
        if (selected.kind === 'project') {
          const project = inventory.projects.find((p) => groupKey(p) === selected.key)
          return project ? <ProjectActions project={project} /> : null
        }
        // A container's verbs are governed by its project's grant, so the project is found first
        // and the panel carries both — the adoption control a refusal would send the operator
        // looking for is in the same place as the buttons that would refuse (FR-017).
        for (const project of inventory.projects) {
          const container = project.containers.find((c) => c.id === selected.id)
          if (container) return <ProjectActions project={project} container={container} />
        }
        return null
      })()}
    </div>
  )
}
