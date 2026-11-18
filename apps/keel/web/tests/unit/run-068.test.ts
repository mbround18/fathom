/** Spec 068 T044, T051, T060, T066: run wording, Start dialog params and required-check input. */
import { describe, expect, it } from 'vitest'

import {
  checkLabel,
  checksSummary,
  checkTone,
  contextProblem,
  forgeLink,
  initialParams,
  MAX_CONTEXTS,
  narrowedText,
  paramProblem,
  paramsReady,
  paramsToSend,
  skipText,
  timeoutText,
  unavailableText,
} from '@/features/runs/run-068'
import type { ParamView, RunCheckView } from '@/protocol/wire'

const DIGEST = `sha256:${'a'.repeat(64)}`
const choice = (over: Partial<ParamView> = {}): ParamView => ({
  kind: 'choice',
  allowed: ['jellyfin', 'keel'],
  default: 'keel',
  unavailable: null,
  ...over,
})
const digest: ParamView = { kind: 'oci_digest', allowed: null, default: null, unavailable: null }
const check = (context: string, state: RunCheckView['state']): RunCheckView => ({ context, state, target_url: null })

describe('required checks on a run', () => {
  it('says every state in words with a tone', () => {
    expect(checkLabel('success')).toBe('passed')
    expect(checkLabel('missing')).toBe('not reported yet')
    expect(checkTone('error')).toBe('err')
    expect(checkTone('pending')).toBe('wait')
    expect(checkTone('success')).toBe('ok')
  })

  it('summarises passed, or failed when any failed', () => {
    expect(checksSummary([check('ci/build', 'success'), check('ci/lint', 'pending')])).toBe('1 of 2 required checks passed')
    expect(checksSummary([check('ci/build', 'failure'), check('ci/lint', 'success')])).toBe('1 of 2 required checks failed')
    expect(checksSummary([check('ci/build', 'success')])).toBe('1 of 1 required check passed')
  })

  it('links only http(s) forge urls', () => {
    expect(forgeLink('https://git.example/ci/1')).toBe('https://git.example/ci/1')
    expect(forgeLink('javascript:alert(1)')).toBeNull()
    expect(forgeLink('not a url')).toBeNull()
    expect(forgeLink(null)).toBeNull()
  })
})

describe('run header and host rows', () => {
  it('names the vars files a run was narrowed by', () => {
    expect(narrowedText(['group_vars/web.yml'])).toBe('Only the hosts this vars file names: group_vars/web.yml')
    expect(narrowedText(['a.yml', 'b.yml'])).toContain('these vars files')
  })

  it('says why a step was skipped', () => {
    expect(skipText('dependency_failed')).toBe('skipped: an earlier step failed')
    expect(skipText(null)).toBe('skipped')
    expect(skipText('host_revoked')).toBe('skipped: host revoked')
  })
})

describe('Start dialog params', () => {
  it('starts a choice at its allowed default, a digest empty', () => {
    expect(initialParams({ app: choice(), image: digest })).toEqual({ app: 'keel', image: '' })
    expect(initialParams({ app: choice({ default: 'gone' }) })).toEqual({ app: '' })
    expect(initialParams(undefined)).toEqual({})
  })

  it('refuses values outside the list, and every value of an unavailable choice', () => {
    expect(paramProblem(choice(), 'keel')).toBeNull()
    expect(paramProblem(choice(), 'other')).toBe('Pick one of the listed values.')
    expect(paramProblem(choice(), '')).toBe('Pick a value.')
    expect(paramProblem(choice({ allowed: null, unavailable: 'choice_from_missing' }), 'keel')).toBe(
      unavailableText('choice_from_missing'),
    )
    expect(paramProblem(choice({ allowed: [] }), '')).toBe('There is nothing to pick from at this commit.')
  })

  it('checks a digest', () => {
    expect(paramProblem(digest, DIGEST)).toBeNull()
    expect(paramProblem(digest, `  ${DIGEST} `)).toBeNull()
    expect(paramProblem(digest, 'sha256:abc')).toContain('64 lowercase hex')
    expect(paramProblem(digest, '')).toBe('Enter an image digest.')
  })

  it('is ready only when every param is valid, and sends them trimmed', () => {
    const params = { app: choice(), image: digest }
    expect(paramsReady(params, { app: 'keel', image: '' })).toBe(false)
    expect(paramsReady(params, { app: 'keel', image: ` ${DIGEST}` })).toBe(true)
    expect(paramsToSend(params, { app: 'keel', image: ` ${DIGEST}`, stray: 'x' })).toEqual({ app: 'keel', image: DIGEST })
    expect(paramsReady(undefined, {})).toBe(true)
  })
})

describe('required checks on a binding', () => {
  it('formats the timeout', () => {
    expect(timeoutText(60)).toBe('1 min')
    expect(timeoutText(3600)).toBe('1 h')
    expect(timeoutText(5400)).toBe('1 h 30 min')
  })

  it('refuses empty, duplicate, control and too many contexts', () => {
    expect(contextProblem('  ', [])).toMatch(/Enter/)
    expect(contextProblem('ci/build', ['ci/build'])).toMatch(/already/)
    expect(contextProblem('ci\u0007x', [])).toMatch(/printable/)
    expect(contextProblem('x'.repeat(201), [])).toMatch(/200/)
    expect(contextProblem('ci/new', Array.from({ length: MAX_CONTEXTS }, (_, i) => `c${String(i)}`))).toMatch(/At most/)
    expect(contextProblem(' ci/build ', [])).toBeNull()
  })
})
