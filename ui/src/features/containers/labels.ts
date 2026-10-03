/**
 * 048-docker-management US1: the words a project group is labelled with.
 *
 * Separate from the list that renders them because these are the decisions worth testing without
 * a DOM: that the no-project bucket is named rather than left blank, that "not adopted" is said to
 * the operator as what it means for them, and that an unreadable definition keeps the parser's own
 * words including the position in them.
 */
import type { AdoptionState, Container, OperationRefusal, Project } from '@/protocol/types'

/**
 * The grant in force right now.
 *
 * `project.adoption` is part of an inventory reading, so it is as old as that reading. The live
 * grant list is the one the backend emits whenever it changes, and FR-015 says an adoption takes
 * effect the moment it is recorded — not at the next refresh. So the list wins for the explicit
 * case, and an implicit grant is kept whole: that one comes from a cluster Fathom manages, is not
 * in the adopted list, and is not the operator's to hold or release.
 */
export function effectiveAdoption(project: Project, adopted: string[]): AdoptionState {
  if (typeof project.adoption === 'object') return project.adoption
  if (project.group.kind !== 'project') return 'not_adopted'
  return adopted.includes(project.group.name) ? 'adopted' : 'not_adopted'
}

/** The name to show. The no-project bucket is named as such, never left blank (FR-006). */
export function groupLabel(project: Project): string {
  return project.group.kind === 'project' ? project.group.name : 'No compose project'
}

/** A stable key and testid fragment for a group. */
/**
 * What the actions panel is pointed at.
 *
 * A project and a container are different scopes with different argvs, so the selection says
 * which it is rather than leaving the panel to guess from the shape of a string.
 */
export type Selection = { kind: 'project'; key: string } | { kind: 'container'; id: string }

export function groupKey(project: Project): string {
  return project.group.kind === 'project' ? project.group.name : 'unowned'
}

/**
 * Whether one container may be mutated.
 *
 * The container's own adoption, not its group's. A node of a cluster Fathom manages carries no
 * compose project, so the bucket it sits in stays read-only while the node itself is actionable
 * (FR-016) — asking the group would make one of those two facts unsayable. A container that does
 * have a project is answered from the live grant, for the same reason [`effectiveAdoption`]
 * exists: a project adopted a moment ago is adopted now, and one released a moment ago is not.
 */
export function containerIsActionable(container: Container, granted: string[]): boolean {
  // A cluster's grant. It is not in `granted` because no project was adopted to put it there, and
  // it cannot be released, so the snapshot is the whole truth for it.
  if (typeof container.adoption === 'object') return true
  // Everything else comes from the live grant and never from the snapshot's own `adopted`: an
  // inventory read while a project was adopted still says so after the project is released, and a
  // row that keeps its verbs after a release is a row that lies about what it may do (FR-015).
  return container.project !== null && granted.includes(container.project)
}

/**
 * The cluster a container's write access comes from, when that is where it comes from.
 *
 * Named rather than implied: "you may stop this" and "because kind made it" are two different
 * things for an operator to know, and the second is the one that explains the first.
 */
export function implicitCluster(container: Container): string | null {
  return typeof container.adoption === 'object'
    ? container.adoption.implicit_via_managed_cluster
    : null
}

/**
 * What the adoption badge says.
 *
 * "Read-only" rather than "not adopted": the operator needs to know what it means for them, and
 * the heading already says which project it is about.
 */
export function adoptionLabel(project: Project): string {
  if (project.adoption === 'adopted') return 'Adopted'
  if (project.adoption === 'not_adopted') return 'Read-only'
  return `Adopted with ${project.adoption.implicit_via_managed_cluster}`
}

/** The sentence that says what the current adoption means for the operator. */
export function adoptionExplanation(project: Project): string {
  if (project.adoption === 'adopted') {
    return 'Adopted for this session. Lifecycle and removal operations on it are available, each behind its own preview.'
  }
  if (project.adoption === 'not_adopted') {
    return 'Read-only. This daemon is shared, so Fathom will not act on a project until you adopt it.'
  }
  return `Write access comes from ${project.adoption.implicit_via_managed_cluster}, a cluster Fathom manages. It is the cluster's grant, so there is nothing here to release.`
}

/** What the definition file line says, or `null` when the project declares none. */
export function definitionLabel(project: Project): string | null {
  const state = project.definition_state
  if (state === 'undeclared') return null
  const path = project.definition_file ?? 'its definition file'
  if (state === 'present') return path
  if (state === 'missing') return `${path} — no longer on disk`
  // The parser's own words, kept whole: the position is the actionable part of them.
  return `${path} — cannot be read: ${state.unreadable}`
}

/*
 * 048 US2: the two previews/outcome strings that are worth asserting without a DOM.
 */

/** Reclaimable size, in the units a person reads. `null` stays unsaid rather than becoming "0 B". */
export function sizeLabel(bytes: number | null): string | null {
  if (bytes === null) return null
  const units = ['B', 'KiB', 'MiB', 'GiB', 'TiB']
  let n = bytes
  let u = 0
  while (n >= 1024 && u < units.length - 1) {
    n /= 1024
    u += 1
  }
  return `${u === 0 ? String(n) : n.toFixed(1)} ${units[u] ?? 'B'}`
}

/** What a refusal says, in the operator's terms rather than the enum's. */
export function refusalLabel(refusal: OperationRefusal): string {
  switch (refusal.kind) {
    case 'not_adopted':
      // The variant carries the project name, not a sentence, so the sentence is built here —
      // mirroring the backend's wording rather than rendering a bare "shop" into the panel.
      return `'${refusal.detail}' has not been adopted, so Fathom will not change it. Adopt it first.`
    case 'preview_stale':
      return 'What is running changed since the preview was taken, so the command it showed is no longer the one that would run. Take a new preview.'
    case 'confirmation_mismatch':
      return 'The typed confirmation did not match, so nothing ran.'
    case 'presence_lost':
      return 'The container runtime is no longer reachable, so nothing ran.'
    // Already a whole sentence by the time it reaches here: the reason is the entire content, and
    // "could not be previewed" on its own is not something an operator can act on.
    case 'not_previewable':
      return refusal.detail
  }
}
