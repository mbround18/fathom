/**
 * Resource Edit Session unit tests (US1, T017): pure-function coverage for `editSession.ts`'s
 * open → edit → diff → apply state machine, independent of the store/React tree.
 */
import { describe, expect, it } from 'vitest'

import { SECTION } from '@/protocol/commands'
import type { ObjectRef } from '@/resources/types'
import {
  applyLiveUpdate,
  buildApplyCommand,
  canApply,
  classifyApplyError,
  discard,
  editYaml,
  markApplied,
  markApplying,
  markFailed,
  openSession,
  parsesAsYaml,
  reviewDiff,
} from '@/store/editSession'

const REF: ObjectRef = { section: SECTION.pods, kind: 'Pod', namespace: 'default', name: 'web-1' }

function rawJson(overrides: Record<string, unknown> = {}): string {
  return JSON.stringify({
    apiVersion: 'v1',
    kind: 'Pod',
    metadata: { name: 'web-1', namespace: 'default', resourceVersion: '100' },
    spec: { containers: [] },
    ...overrides,
  })
}

describe('openSession', () => {
  it('parses apiVersion and resourceVersion from raw_json', () => {
    const session = openSession(REF, rawJson())
    expect(session.target).toEqual({
      apiVersion: 'v1',
      kind: 'Pod',
      namespace: 'default',
      name: 'web-1',
    })
    expect(session.openedResourceVersion).toBe('100')
    expect(session.status).toBe('editing')
    expect(session.dirty).toBe(false)
    expect(session.editedYaml).toBe(session.liveYaml)
  })

  it('falls back to empty strings when apiVersion/resourceVersion are missing', () => {
    const session = openSession(REF, JSON.stringify({ kind: 'Pod', metadata: {} }))
    expect(session.target.apiVersion).toBe('')
    expect(session.openedResourceVersion).toBe('')
  })
})

describe('editYaml', () => {
  it('marks the session dirty once the buffer diverges from liveYaml, and clean again if reverted', () => {
    const session = openSession(REF, rawJson())
    const edited = editYaml(session, session.liveYaml + '\n# comment')
    expect(edited.dirty).toBe(true)
    const reverted = editYaml(edited, session.liveYaml)
    expect(reverted.dirty).toBe(false)
  })
})

describe('parsesAsYaml / canApply', () => {
  it('rejects empty content and tab indentation', () => {
    expect(parsesAsYaml('')).toBe(false)
    expect(parsesAsYaml('   \n  ')).toBe(false)
    expect(parsesAsYaml('\tfoo: bar')).toBe(false)
  })

  it('rejects unbalanced quotes', () => {
    expect(parsesAsYaml('name: "unterminated')).toBe(false)
    expect(parsesAsYaml("name: 'unterminated")).toBe(false)
  })

  it('accepts well-formed content', () => {
    expect(parsesAsYaml('kind: Pod\nmetadata:\n  name: web-1\n')).toBe(true)
  })

  it('canApply requires both dirty and syntactically valid', () => {
    const session = openSession(REF, rawJson())
    expect(canApply(session)).toBe(false) // not dirty yet
    const brokenEdit = editYaml(session, '\tbroken')
    expect(canApply(brokenEdit)).toBe(false) // dirty but invalid
    const validEdit = editYaml(session, session.liveYaml + '\n# comment')
    expect(canApply(validEdit)).toBe(true)
  })
})

describe('applyLiveUpdate', () => {
  it('is a no-op when the live payload is unchanged', () => {
    const session = openSession(REF, rawJson())
    const same = applyLiveUpdate(session, rawJson())
    expect(same).toBe(session)
  })

  it('recomputes liveYaml and flags staleness when the live object changed', () => {
    const session = openSession(REF, rawJson())
    const reviewed = reviewDiff(session)
    expect(reviewed.liveYamlChangedSinceReview).toBe(false)

    const updated = applyLiveUpdate(
      reviewed,
      rawJson({ metadata: { name: 'web-1', namespace: 'default', resourceVersion: '101' } }),
    )
    expect(updated).not.toBe(reviewed)
    expect(updated.liveYamlChangedSinceReview).toBe(true)
    expect(updated.liveYaml).not.toBe(reviewed.liveYaml)
  })
})

describe('reviewDiff / discard', () => {
  it('reviewDiff moves to diffing and clears the staleness flag', () => {
    const session = { ...openSession(REF, rawJson()), liveYamlChangedSinceReview: true }
    const reviewed = reviewDiff(session)
    expect(reviewed.status).toBe('diffing')
    expect(reviewed.liveYamlChangedSinceReview).toBe(false)
  })

  it('discard reverts the buffer to the current liveYaml and clears dirty/error state', () => {
    const session = openSession(REF, rawJson())
    const edited = editYaml(session, session.liveYaml + '\n# scratch')
    const failed = markFailed(edited, 'conflict: stale resourceVersion')
    const discarded = discard(failed)
    expect(discarded.editedYaml).toBe(discarded.liveYaml)
    expect(discarded.dirty).toBe(false)
    expect(discarded.status).toBe('editing')
    expect(discarded.lastError).toBeUndefined()
  })
})

describe('classifyApplyError / markFailed', () => {
  it('classifies a "conflict:" prefix as conflict', () => {
    expect(classifyApplyError('conflict: resourceVersion mismatch')).toBe('conflict')
  })

  it('classifies a "not-found:" prefix as deleted', () => {
    expect(classifyApplyError('not-found: pods "web-1" not found')).toBe('deleted')
  })

  it('classifies anything else as a generic error', () => {
    expect(classifyApplyError('validation: invalid YAML')).toBe('error')
    expect(classifyApplyError('permission denied')).toBe('error')
  })

  it('markFailed preserves the edit buffer and sets status + verbatim message', () => {
    const session = openSession(REF, rawJson())
    const edited = editYaml(session, session.liveYaml + '\n# my-change')
    const applying = markApplying(edited, 42)
    expect(applying.status).toBe('applying')
    expect(applying.requestId).toBe(42)

    const conflict = markFailed(applying, 'conflict: resourceVersion mismatch')
    expect(conflict.status).toBe('conflict')
    expect(conflict.lastError).toBe('conflict: resourceVersion mismatch')
    expect(conflict.editedYaml).toBe(applying.editedYaml) // buffer preserved (T014)

    const deleted = markFailed(applying, 'not-found: pods "web-1" not found')
    expect(deleted.status).toBe('deleted')
    expect(deleted.editedYaml).toBe(applying.editedYaml) // buffer preserved (T015)
    expect(deleted.status).not.toBe(conflict.status) // never conflated (C2)
  })
})

describe('markApplied', () => {
  it('adopts the applied buffer as the new liveYaml and clears dirty/error state', () => {
    const session = openSession(REF, rawJson())
    const edited = editYaml(session, session.liveYaml + '\n# applied-change')
    const applying = markApplying(edited, 7)
    const applied = markApplied(applying, '101')

    expect(applied.status).toBe('applied')
    expect(applied.openedResourceVersion).toBe('101')
    expect(applied.liveYaml).toBe(edited.editedYaml)
    expect(applied.editedYaml).toBe(edited.editedYaml)
    expect(applied.dirty).toBe(false)
    expect(applied.lastError).toBeUndefined()
  })
})

describe('buildApplyCommand', () => {
  it('builds an ApplyResourceYaml command from the session target + edit buffer', () => {
    const session = openSession(REF, rawJson())
    const edited = editYaml(session, session.liveYaml + '\n# change')
    const cmd = buildApplyCommand(edited)
    expect(cmd).toEqual({
      type: 'ApplyResourceYaml',
      data: {
        api_version: 'v1',
        kind: 'Pod',
        namespace: 'default',
        name: 'web-1',
        yaml: edited.editedYaml,
        expected_resource_version: '100',
      },
    })
  })
})
