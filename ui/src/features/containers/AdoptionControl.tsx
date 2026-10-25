/**
 * 048-docker-management US2: the adopt / release control.
 *
 * The rule this component exists to keep (FR-017) is that the control is never silently absent.
 * A project the operator has not adopted is read-only, and the honest way to say so is to say it
 * next to the offer that changes it — not to hide the buttons and let the operator conclude that
 * Fathom cannot do it at all. So there are exactly three renderings, and all three say something:
 *
 *   adopted                 — what it means, and the way back
 *   not adopted             — that it is read-only, and the way in
 *   implicitly adopted      — which cluster lends it write access, and that releasing is not ours
 *                             to offer, because the grant is the cluster's and not the operator's
 *
 * The no-compose-project bucket is not a project and cannot be adopted: a grant over it would
 * cover every unrelated stray on the daemon at once. It says so, rather than showing a button
 * that would refuse.
 */
import { LockSimpleOpenIcon, LockSimpleIcon } from '@phosphor-icons/react'

import { Button } from '@companion/ui'
import type { Container, Project } from '@/protocol/types'
import { useStore } from '@/store'

import { adoptionExplanation, effectiveAdoption, groupKey, implicitCluster } from './labels'

export function AdoptionControl({
  project,
  container,
}: {
  project: Project
  /** The selected container, when one is — its rights may differ from its group's (FR-016). */
  container?: Container
}) {
  const adopt = useStore((s) => s.adoptContainerProject)
  const release = useStore((s) => s.releaseContainerProject)
  const fellBack = useStore((s) => s.containerRuntime.adoptionFellBack)
  const granted = useStore((s) => s.containerRuntime.adopted)

  const key = groupKey(project)
  const named = project.group.kind === 'project' ? project.group.name : null
  const adoption = effectiveAdoption(project, granted)
  // A node of a managed cluster is actionable inside a bucket that is not, so the sentence here
  // has to be about the thing selected rather than about the bucket it happens to sit in.
  const fromCluster = container === undefined ? null : implicitCluster(container)
  const implicit = typeof adoption === 'object'

  return (
    <div
      data-testid={`adoption-control-${key}`}
      data-adoption={implicit || fromCluster !== null ? 'implicit' : adoption}
      className="flex min-w-0 flex-col gap-2 border border-border bg-card/40 p-3 sm:flex-row sm:items-center"
    >
      <div className="min-w-0 flex-1">
        <p data-testid={`adoption-explanation-${key}`} className="text-xs text-muted-foreground">
          {fromCluster !== null
            ? `${container?.name ?? 'This container'} backs ${fromCluster}, a cluster Fathom manages, so it is actionable without a grant. That is the cluster's write access, not this bucket's: everything else here stays read-only, and there is nothing to release.`
            : named === null
              ? 'Containers outside a compose project are read-only. This bucket is not a unit Fathom can adopt — a grant over it would cover unrelated strays Fathom knows nothing about. A container backing a cluster Fathom manages is actionable on its own.'
              : adoptionExplanation({ ...project, adoption })}
        </p>
        {/* FR-013: a grant record Fathom could not read is the operator's business, because every
            refusal they are about to see comes from it. */}
        {fellBack && (
          <p data-testid={`adoption-fell-back-${key}`} className="mt-1 text-xs text-status-warn">
            The saved adoptions could not be read, so this session started with none. Anything you
            adopt now lasts until Fathom closes.
          </p>
        )}
      </div>
      {named !== null && !implicit && (
        <Button
          size="sm"
          variant={adoption === 'adopted' ? 'ghost' : 'default'}
          data-testid={`adoption-toggle-${key}`}
          onClick={() => {
            if (adoption === 'adopted') release(named)
            else adopt(named)
          }}
        >
          {adoption === 'adopted' ? (
            <>
              <LockSimpleIcon aria-hidden="true" />
              Release {named}
            </>
          ) : (
            <>
              <LockSimpleOpenIcon aria-hidden="true" />
              Adopt {named}
            </>
          )}
        </Button>
      )}
    </div>
  )
}
