/**
 * Helm Mutation preview state (006-helm-mutate, T009): pure helpers for the rollback/upgrade
 * preview → apply flow, mirroring `editSession.ts`'s `'diffing' → 'applying'` shape (research.md
 * R2) but kept as its own module since the domain differs (Helm releases, not arbitrary
 * resources) and US1/US2/US3 each drive it from a different command pair.
 */
import type { HelmMutationPreviewReady } from '@/protocol/types'

export type HelmMutationStatus =
  | 'idle'
  | 'previewing'
  | 'preview_ready'
  | 'preview_failed'
  | 'applying'
  | 'applied'
  | 'apply_failed'

export interface HelmMutationSession {
  status: HelmMutationStatus
  preview?: HelmMutationPreviewReady
  lastError?: string
}

export const initialHelmMutationSession: HelmMutationSession = { status: 'idle' }

export function startPreview(): HelmMutationSession {
  return { status: 'previewing' }
}

export function previewReady(preview: HelmMutationPreviewReady): HelmMutationSession {
  return { status: 'preview_ready', preview }
}

export function previewFailed(message: string): HelmMutationSession {
  return { status: 'preview_failed', lastError: message }
}

export function startApply(session: HelmMutationSession): HelmMutationSession {
  return { ...session, status: 'applying' }
}

export function applied(): HelmMutationSession {
  return { status: 'applied' }
}

export function applyFailed(session: HelmMutationSession, message: string): HelmMutationSession {
  return { ...session, status: 'apply_failed', lastError: message }
}

export function reset(): HelmMutationSession {
  return initialHelmMutationSession
}
