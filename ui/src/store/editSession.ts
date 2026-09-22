/**
 * Resource Edit Session (data-model.md "Resource Edit Session", US1 tasks T021-T024): pure
 * helpers for the YAML tab's edit → diff → apply flow. Kept out of `store/index.ts` so that
 * file's own edit stays small and additive, and so `yaml-diff.test.ts` can exercise the
 * transitions directly without a store.
 */
import type { BackendCmd } from '@/protocol/types'
import type { ObjectRef } from '@/resources/types'
import { jsonToYaml } from '@/resources/yaml'

/** The identity the session was opened for; immutable for the life of the session (FR-005). */
export interface ResourceEditTarget {
  apiVersion: string
  kind: string
  namespace: string | null
  name: string
}

export type EditSessionStatus =
  | 'editing'
  | 'diffing'
  | 'applying'
  | 'conflict'
  | 'error'
  | 'applied'
  /**
   * NEW beyond data-model.md's status table: the object was deleted server-side after the tab
   * opened (404 on apply). Kept distinct from `'conflict'` (409, stale `resourceVersion`) so the
   * two are never conflated in the UI (spec analyze finding C2).
   */
  | 'deleted'

export interface ResourceEditSession {
  target: ResourceEditTarget
  /** `metadata.resourceVersion` captured when the tab was opened; sent back on apply. */
  openedResourceVersion: string
  /** Rendered via `jsonToYaml()`; re-derived on every live update while the session is open. */
  liveYaml: string
  /** The operator's edit buffer. */
  editedYaml: string
  /** `editedYaml !== liveYaml`; drives the discard-warning (FR-006). */
  dirty: boolean
  /** Set when `liveYaml` changes after the operator last opened the diff view. */
  liveYamlChangedSinceReview: boolean
  status: EditSessionStatus
  /** Verbatim backend error message on `'error'`/`'conflict'`/`'deleted'` (constitution I). */
  lastError?: string
  /** The in-flight (or last) apply's request id, for correlating a late result. */
  requestId?: number
}

function parseField<T>(raw: string, pick: (v: Record<string, unknown>) => T | undefined): T | undefined {
  try {
    const value: unknown = JSON.parse(raw)
    if (typeof value !== 'object' || value === null) return undefined
    return pick(value as Record<string, unknown>)
  } catch {
    return undefined
  }
}

function parseResourceVersion(raw: string): string {
  return (
    parseField(raw, (v) => {
      const metadata = v.metadata
      if (typeof metadata !== 'object' || metadata === null) return undefined
      const rv = (metadata as Record<string, unknown>).resourceVersion
      return typeof rv === 'string' ? rv : undefined
    }) ?? ''
  )
}

function parseApiVersion(raw: string): string {
  return (
    parseField(raw, (v) => (typeof v.apiVersion === 'string' ? v.apiVersion : undefined)) ?? ''
  )
}

/** A fresh session for `ref`, seeded from the same `raw_json` the read-only YAML tab rendered. */
export function openSession(ref: ObjectRef, raw: string): ResourceEditSession {
  const yaml = jsonToYaml(raw)
  return {
    target: {
      apiVersion: parseApiVersion(raw),
      kind: ref.kind,
      namespace: ref.namespace,
      name: ref.name,
    },
    openedResourceVersion: parseResourceVersion(raw),
    liveYaml: yaml,
    editedYaml: yaml,
    dirty: false,
    liveYamlChangedSinceReview: false,
    status: 'editing',
  }
}

/** T024: a watch-driven `raw_json` update for the object the session is open on. */
export function applyLiveUpdate(session: ResourceEditSession, raw: string): ResourceEditSession {
  const liveYaml = jsonToYaml(raw)
  if (liveYaml === session.liveYaml) return session
  return { ...session, liveYaml, liveYamlChangedSinceReview: true }
}

export function editYaml(session: ResourceEditSession, editedYaml: string): ResourceEditSession {
  return { ...session, editedYaml, dirty: editedYaml !== session.liveYaml }
}

/**
 * Basic client-side syntax check only (data-model.md validation rules) — full structural
 * validation happens backend-side (`src/backend/apply.rs::parse_and_validate_yaml`). This
 * rejects only the unambiguous paste mistakes (unbalanced quotes, tab indentation, empty
 * content) rather than attempting a real YAML grammar.
 */
export function parsesAsYaml(yaml: string): boolean {
  if (yaml.trim().length === 0) return false
  if (/^\t/m.test(yaml)) return false
  const singleQuotes = (yaml.match(/'/g) ?? []).length
  const doubleQuotes = (yaml.match(/"/g) ?? []).length
  return singleQuotes % 2 === 0 && doubleQuotes % 2 === 0
}

/** Apply is disabled unless the buffer changed and passes the basic syntax check. */
export function canApply(session: ResourceEditSession): boolean {
  return session.dirty && parsesAsYaml(session.editedYaml)
}

/** Opens the diff view and marks the current `liveYaml` as reviewed. */
export function reviewDiff(session: ResourceEditSession): ResourceEditSession {
  return { ...session, status: 'diffing', liveYamlChangedSinceReview: false }
}

/**
 * Discard the edit buffer (T026: navigate-away confirmation, or an explicit "discard" action).
 * `editedYaml` reverts to the *current* `liveYaml`, not a frozen snapshot from when the tab
 * opened, since the object may have changed underneath while the operator was editing.
 */
export function discard(session: ResourceEditSession): ResourceEditSession {
  return {
    ...session,
    editedYaml: session.liveYaml,
    dirty: false,
    status: 'editing',
    lastError: undefined,
  }
}

export function markApplying(session: ResourceEditSession, requestId: number): ResourceEditSession {
  return { ...session, status: 'applying', requestId, lastError: undefined }
}

/**
 * Classifies a `CommandFinished` failure's verbatim message by the stable prefix
 * `src/backend/apply.rs::ApplyError`'s `Display` impl documents ("conflict: "/"not-found: ").
 * Any other message (validation, permission, discovery failure, ...) is `'error'`.
 */
export function classifyApplyError(message: string): 'conflict' | 'deleted' | 'error' {
  if (message.startsWith('conflict:')) return 'conflict'
  if (message.startsWith('not-found:')) return 'deleted'
  return 'error'
}

/**
 * On failure the edit buffer is preserved unchanged (FR-007 for conflict; same for the
 * deleted-mid-edit case, T015): the operator must explicitly re-open to refresh `liveYaml`/
 * `openedResourceVersion` before retrying.
 */
export function markFailed(session: ResourceEditSession, message: string): ResourceEditSession {
  return { ...session, status: classifyApplyError(message), lastError: message }
}

export function markApplied(
  session: ResourceEditSession,
  resourceVersion: string,
): ResourceEditSession {
  return {
    ...session,
    status: 'applied',
    openedResourceVersion: resourceVersion,
    liveYaml: session.editedYaml,
    editedYaml: session.editedYaml,
    dirty: false,
    lastError: undefined,
  }
}

/** The `ApplyResourceYaml` command for `session`'s current edit buffer. */
export function buildApplyCommand(session: ResourceEditSession): BackendCmd {
  return {
    type: 'ApplyResourceYaml',
    data: {
      api_version: session.target.apiVersion,
      kind: session.target.kind,
      namespace: session.target.namespace,
      name: session.target.name,
      yaml: session.editedYaml,
      expected_resource_version: session.openedResourceVersion,
    },
  }
}
