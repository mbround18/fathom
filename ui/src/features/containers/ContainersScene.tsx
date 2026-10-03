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
import { CreateContainerForm } from './CreateForms'
import { ContainerStreams } from './ContainerStreams'
import { groupKey, type Selection } from './labels'
import { OperationPreview } from './OperationPreview'
import { ProjectActions } from './ProjectActions'
import { UsagePanel } from './UsagePanel'

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
  const operation = useStore((s) => s.containerRuntime.operation)
  const [selected, setSelected] = useState<Selection | null>(null)
  const [creating, setCreating] = useState(false)

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
          variant={creating ? 'default' : 'ghost'}
          data-testid="containers-create"
          disabled={operation !== undefined}
          onClick={() => {
            setCreating((c) => !c)
          }}
        >
          Run a container
        </Button>
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

      {/* Form-driven and nothing more (FR-019): the form collects fields and ends at a preview,
          and the preview that follows is where the argv is approved. */}
      {creating && operation === undefined && (
        <CreateContainerForm
          onDone={() => {
            setCreating(false)
          }}
        />
      )}

      {/* A creation belongs to no row, so its preview cannot live beside one: with nothing
          selected this is the only place it can appear. When a row *is* selected, `ProjectActions`
          renders it there instead — one preview on screen, never two (FR-029). */}
      {operation !== undefined && selected === null && (
        <div className="shrink-0 border-t border-border bg-background/60 p-3">
          <OperationPreview />
        </div>
      )}

      {/* A floor under the inventory. Everything below is mounted by a selection, and at laptop
          height a log follow (16rem) over the usage figures took the whole pane: the virtualized
          list measured zero and rendered no rows, so selecting a container made the inventory it
          was selected from disappear. The list keeps its floor and the panes below scroll. */}
      <div className="flex min-h-28 min-w-0 flex-1 flex-col">
        <ContainerGroupList
          projects={inventory.projects}
          selected={selected}
          onSelect={(next) => {
            // Changing selection drops whatever preview was open: a preview belongs to the
            // operation it was taken for, and one left on screen beside another object's verbs is
            // the kind of mismatch FR-018 exists to prevent.
            setSelected((current) => (sameSelection(current, next) ? null : next))
            dismiss()
          }}
        />
      </div>

      {/* The selected object's verbs live outside the virtualized list: a row whose height grows
          when it is chosen would move every row below it. Bounded and scrolling, so the panes a
          selection mounts can never be taller than the inventory they belong to. */}
      <div className="flex max-h-[60%] min-w-0 shrink-0 flex-col overflow-y-auto">
        {(() => {
          if (selected === null) return null
          if (selected.kind === 'project') {
            const project = inventory.projects.find((p) => groupKey(p) === selected.key)
            if (!project) return null
            return (
              <>
                <ProjectActions project={project} />
                {/* Usage is the project read as one unit, which is what grouping was for. It is
                  mounted under the selection, so leaving the selection stops the sampling
                  (FR-045) without anything here having to say so. */}
                <UsagePanel key={selected.key} containers={project.containers} />
              </>
            )
          }
          // A container's verbs are governed by its project's grant, so the project is found first
          // and the panel carries both — the adoption control a refusal would send the operator
          // looking for is in the same place as the buttons that would refuse (FR-017).
          for (const project of inventory.projects) {
            const container = project.containers.find((c) => c.id === selected.id)
            if (container) {
              return (
                <>
                  <ProjectActions project={project} container={container} />
                  {/* One container's own figures, sampled only for it. */}
                  <UsagePanel key={container.id} containers={[container]} />
                  {/* Reading and attaching are the container's own, not the project's: logs are
                    offered for every container (FR-035) and a shell only where the project's
                    grant allows a change (FR-013). */}
                  <ContainerStreams key={container.id} container={container} />
                </>
              )
            }
          }
          return null
        })()}
      </div>
    </div>
  )
}
