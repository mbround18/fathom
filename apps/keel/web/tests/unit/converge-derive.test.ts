/** Spec 068 T030: the converge matrix, state and wording derivations. */
import { describe, expect, it } from 'vitest'

import {
  canConverge,
  confirmLabel,
  convergeBlocked,
  finished,
  hostConvergeText,
  jobsOf,
  planMatrix,
  policyText,
  refusalText,
  retryableSteps,
  runIds,
  stepLabel,
  stepTone,
  ttlText,
  viaText,
  viewMatrix,
} from '@/features/converge/derive'
import type {
  ConvergeHostState,
  ConvergePlan,
  ConvergeStepView,
  ConvergeView,
  HostInfo,
  HostPlan,
  Me,
  PlannedStep,
  RunInfo,
  StepState,
} from '@/protocol/wire'

const step = (position: number, job: string, over: Partial<PlannedStep> = {}): PlannedStep => ({
  position,
  job,
  via: [],
  params: {},
  credential_names: [],
  ...over,
})

const hostPlan = (host: string, steps: PlannedStep[], over: Partial<HostPlan> = {}): HostPlan => ({
  host_id: `host-${host}`,
  host,
  profiles: ['base'],
  steps,
  refusals: [],
  ...over,
})

const plan = (hosts: HostPlan[]): ConvergePlan => ({
  binding_id: 'b',
  commit: 'a'.repeat(40),
  inventory_revision: 'rev-1',
  check: false,
  hosts,
  policy: { requires_approval: false, approvals_needed: 0, four_eyes: false, approval_ttl_secs: 3600 },
  review: [],
  replaces: [],
  blocked_by: [],
})

const viewStep = (position: number, job: string, state: StepState): ConvergeStepView => ({
  position,
  job,
  run_id: `child-${job}-${String(position)}`,
  state,
  reason: null,
  started_at: null,
  ended_at: null,
})

const view = (hosts: { host: string; state: ConvergeHostState; steps: ConvergeStepView[] }[]): ConvergeView => ({
  run: { id: 'conv-1' } as RunInfo,
  hosts: hosts.map((h) => ({ host_id: `host-${h.host}`, ...h })),
  retry_of: null,
})

const host = (over: Partial<HostInfo>): HostInfo =>
  ({ id: 'h', workspace: 'ws', inventory_name: 'alpha', state: 'online', profiles: ['base', 'web'], ...over }) as HostInfo

const me = (over: Partial<Me>): Me => ({ id: 'u', instance_role: 'user', grants: [], ...over }) as Me

describe('the plan', () => {
  const p = plan([
    hostPlan('alpha', [step(1, 'debian', { params: { tz: 'UTC', a: '1' }, credential_names: ['z', 'k'] }), step(2, 'web')]),
    hostPlan('bravo', [step(1, 'debian')]),
    hostPlan('charlie', [step(1, 'debian')], { refusals: [{ job: 'web', reason: 'outside_ceiling' }] }),
    hostPlan('delta', []),
  ])

  it('lays hosts out by step position and marks who Confirm includes', () => {
    const m = planMatrix(p)
    expect(m.positions).toEqual([1, 2])
    expect(m.rows.map((r) => r.cells.map((c) => c?.job ?? null))).toEqual([
      ['debian', 'web'],
      ['debian', null],
      ['debian', null],
      [null, null],
    ])
    expect(m.rows.map((r) => r.included)).toEqual([true, true, false, false])
  })

  it('counts only included hosts for Confirm', () => {
    expect(confirmLabel(p)).toBe('Converge 2 hosts (3 steps)')
    expect(confirmLabel(plan([hostPlan('a', [step(1, 'x')])]))).toBe('Converge 1 host (1 step)')
  })

  it('lists each job once with sorted params and credential names', () => {
    expect(jobsOf(p)).toEqual([
      { job: 'debian', params: [['a', '1'], ['tz', 'UTC']], credential_names: ['k', 'z'], hosts: 2 },
      { job: 'web', params: [], credential_names: [], hosts: 1 },
    ])
  })

  it('says where a step came from', () => {
    expect(viaText(step(1, 'debian', { via: ['keel-node', 'debian'] }))).toBe('from keel-node → debian')
    expect(viaText(step(1, 'debian'))).toBe('assigned directly')
  })

  it('words the approval policy and its expiry', () => {
    expect(policyText({ requires_approval: false, approvals_needed: 0, four_eyes: false, approval_ttl_secs: 0 }, 3)).toBe(
      'No approval needed: it starts on Confirm.',
    )
    expect(policyText({ requires_approval: true, approvals_needed: 2, four_eyes: true, approval_ttl_secs: 14_400 }, 3)).toBe(
      'Needs 2 approvals, four eyes, expires in 4 h (strictest of 3 steps)',
    )
    expect(policyText({ requires_approval: true, approvals_needed: 1, four_eyes: false, approval_ttl_secs: 1800 }, 1)).toBe(
      'Needs 1 approval, expires in 30 min',
    )
    expect([ttlText(172_800), ttlText(5_400), ttlText(45)]).toEqual(['2 d', '1.5 h', '45 s'])
  })

  it('words refusals, naming the job where it matters', () => {
    expect(refusalText('k3s', 'outside_ceiling')).toBe('k3s: outside the host’s ceiling')
    expect(refusalText('k3s', 'host_revoked')).toBe('the host is revoked')
    expect(refusalText('k3s', 'no_steps')).toBe('its profiles have no steps')
  })
})

describe('the view', () => {
  const v = view([
    { host: 'alpha', state: 'succeeded', steps: [viewStep(1, 'debian', 'succeeded'), viewStep(2, 'web', 'covered')] },
    { host: 'bravo', state: 'failed', steps: [viewStep(1, 'debian', 'failed'), viewStep(2, 'web', 'skipped')] },
    { host: 'charlie', state: 'cancelled', steps: [viewStep(1, 'debian', 'cancelled')] },
  ])

  it('lays hosts out by step position', () => {
    const m = viewMatrix(v)
    expect(m.positions).toEqual([1, 2])
    expect(m.rows.map((r) => [r.host, r.state, r.cells.map((c) => c?.state ?? null)])).toEqual([
      ['alpha', 'succeeded', ['succeeded', 'covered']],
      ['bravo', 'failed', ['failed', 'skipped']],
      ['charlie', 'cancelled', ['cancelled', null]],
    ])
  })

  it('counts the steps Retry failed would run and knows when every chain has ended', () => {
    expect(retryableSteps(v)).toBe(3)
    expect(finished(v)).toBe(true)
    expect(finished(view([{ host: 'a', state: 'holding', steps: [viewStep(1, 'x', 'running')] }]))).toBe(false)
    expect(finished(view([{ host: 'a', state: 'waiting', steps: [viewStep(1, 'x', 'queued')] }]))).toBe(false)
  })

  it('names the parent and every child run', () => {
    expect([...runIds(v)].sort()).toEqual(['child-debian-1', 'child-web-2', 'conv-1'])
  })

  it('labels and tones each step state', () => {
    expect(stepLabel('skipped')).toBe('Skipped: dependency failed')
    expect(stepLabel('running')).toBe('Running')
    expect((['succeeded', 'covered', 'failed', 'lost', 'refused', 'running', 'sent', 'blocked', 'cancelled'] as const).map(stepTone)).toEqual([
      'ok',
      'ok',
      'err',
      'err',
      'err',
      'live',
      'live',
      'muted',
      'muted',
    ])
  })
})

describe('the Hosts column', () => {
  const steps = [viewStep(1, 'debian', 'succeeded'), viewStep(2, 'web', 'running')]
  it('says where the host is in its converge', () => {
    expect(hostConvergeText({ run_id: 'r', state: 'waiting', step: 0, of: 3 })).toBe('waiting · 3 steps')
    expect(hostConvergeText({ run_id: 'r', state: 'waiting', step: 0, of: 1 })).toBe('waiting · 1 step')
    expect(hostConvergeText({ run_id: 'r', state: 'holding', step: 2, of: 3 })).toBe('converging 2/3')
    expect(hostConvergeText({ run_id: 'r', state: 'holding', step: 2, of: 3 }, steps)).toBe('converging 2/3 · web')
    expect(hostConvergeText({ run_id: 'r', state: 'succeeded', step: 3, of: 3 })).toBe('converged')
    expect(hostConvergeText({ run_id: 'r', state: 'failed', step: 1, of: 3 }, steps)).toBe('failed at debian')
    expect(hostConvergeText({ run_id: 'r', state: 'failed', step: 1, of: 3 })).toBe('failed at step 1/3')
    expect(hostConvergeText({ run_id: 'r', state: 'cancelled', step: 1, of: 3 })).toBe('converge cancelled')
  })
})

describe('who may converge', () => {
  const grant = (role: string, profiles: string[], workspace = 'ws') =>
    ({ workspace, workspace_slug: workspace, role, profiles }) as Me['grants'][number]

  it('lets owners and admins converge anything', () => {
    expect(canConverge(me({ instance_role: 'owner' }), host({}))).toBe(true)
    expect(canConverge(me({ instance_role: 'admin' }), host({}))).toBe(true)
    expect(canConverge(null, host({}))).toBe(false)
  })

  it('needs operate grants that together cover every profile of the host', () => {
    expect(canConverge(me({ grants: [grant('operator', [])] }), host({}))).toBe(true)
    expect(canConverge(me({ grants: [grant('operator', ['base'])] }), host({}))).toBe(false)
    expect(canConverge(me({ grants: [grant('operator', ['base']), grant('ws_admin', ['web'])] }), host({}))).toBe(true)
    expect(canConverge(me({ grants: [grant('viewer', [])] }), host({}))).toBe(false)
    expect(canConverge(me({ grants: [grant('operator', [], 'other')] }), host({}))).toBe(false)
  })

  it('gives the reason Converge… is off', () => {
    const op = me({ grants: [grant('operator', ['base'])] })
    expect(convergeBlocked(op, [host({ profiles: ['base'] })], null)).toBe('Bind the workspace’s repository first.')
    expect(convergeBlocked(op, [], 'b')).toBe('Select hosts to converge.')
    expect(convergeBlocked(op, [host({})], 'b')).toBe('You can’t operate on every profile of alpha.')
    expect(convergeBlocked(op, [host({ profiles: ['base'], state: 'revoked' })], 'b')).toBe('alpha is revoked.')
    expect(convergeBlocked(op, [host({ profiles: ['base'] })], 'b')).toBeNull()
  })
})
