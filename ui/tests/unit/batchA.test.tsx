// `@/store` first: see the import-order note in actions.test.ts.
import '@/store'
import { describe, expect, it } from 'vitest'

import { descriptorFor, type ActionDescriptor } from '@/resources'
import { cronjobs } from '@/resources/cronjobs'
import { jobStatus, jobs } from '@/resources/jobs'
import { replicasets } from '@/resources/replicasets'
import { replicationcontrollers } from '@/resources/replicationcontrollers'
import type { CronJobSummary, JobSummary, WorkloadSummary } from '@/protocol/types'

const job = (o: Partial<JobSummary> = {}): JobSummary => ({
  namespace: 'ns',
  name: 'j',
  completions: '0/1',
  duration: '1m',
  active: 0,
  succeeded: 0,
  failed: 0,
  age: '1m',
  ...o,
})
const cron = (o: Partial<CronJobSummary> = {}): CronJobSummary => ({
  namespace: 'ns',
  name: 'c',
  schedule: '*/5 * * * *',
  suspend: false,
  active: 0,
  last_schedule: null,
  age: '1d',
  ...o,
})
const wl = (): WorkloadSummary => ({
  namespace: 'ns',
  name: 'w',
  ready: 1,
  desired: 2,
  updated: 1,
  available: 1,
})
function act<R>(d: { actions: ActionDescriptor<R>[] }, id: string): ActionDescriptor<R> {
  const a = d.actions.find((x) => x.id === id)
  if (!a) throw new Error(`no ${id}`)
  return a
}

describe('batch A registration', () => {
  it.each([
    'workloads/jobs',
    'workloads/cronjobs',
    'workloads/replicasets',
    'workloads/replicationcontrollers',
  ] as const)('%s resolves and is namespaced', (s) => {
    expect(descriptorFor(s)?.namespaced).toBe(true)
  })
})

describe('jobs', () => {
  it('derives status', () => {
    expect(jobStatus(job({ active: 1 }))).toBe('Running')
    expect(jobStatus(job({ succeeded: 1 }))).toBe('Complete')
    expect(jobStatus(job({ failed: 2 }))).toBe('Failed')
    expect(jobStatus(job())).toBe('Pending')
  })
  it('only offers delete, sending DeleteJob', () => {
    expect(jobs.actions.map((a) => a.id)).toEqual(['delete'])
    expect(act(jobs, 'delete').plan([job()], {})[0]?.cmd).toEqual({
      type: 'DeleteJob',
      data: { namespace: 'ns', name: 'j' },
    })
  })
})

describe('cronjobs', () => {
  it('trigger sends TriggerCronJob', () => {
    expect(act(cronjobs, 'trigger').plan([cron()], {})[0]?.cmd).toEqual({
      type: 'TriggerCronJob',
      data: { namespace: 'ns', name: 'c' },
    })
  })
  it('suspend/resume only target rows in the opposite state', () => {
    const rows = [cron({ name: 'a' }), cron({ name: 'b', suspend: true })]
    expect(
      act(cronjobs, 'suspend')
        .plan(rows, {})
        .map((s) => s.cmd),
    ).toEqual([{ type: 'SuspendCronJob', data: { namespace: 'ns', name: 'a', suspend: true } }])
    expect(
      act(cronjobs, 'resume')
        .plan(rows, {})
        .map((s) => s.cmd),
    ).toEqual([{ type: 'SuspendCronJob', data: { namespace: 'ns', name: 'b', suspend: false } }])
  })
  it('delete is destructive and confirmed', () => {
    const d = act(cronjobs, 'delete')
    expect(d.destructive).toBe(true)
    expect(d.prompt).toBe('confirm')
  })
})

describe('replicasets / replicationcontrollers', () => {
  it('offer scale, restart, delete with the right commands', () => {
    const cases = [
      [replicasets, 'ReplicaSet'],
      [replicationcontrollers, 'ReplicationController'],
    ] as const
    for (const [d, k] of cases) {
      expect(d.actions.map((a) => a.id)).toEqual(['scale', 'restart', 'delete'])
      const rows = [wl()]
      expect(act(d, 'scale').plan(rows, { replicas: 3 })[0]?.cmd).toEqual({
        type: `Scale${k}`,
        data: { namespace: 'ns', name: 'w', replicas: 3 },
      })
      expect(act(d, 'restart').plan(rows, {})[0]?.cmd.type).toBe(`Restart${k}`)
      expect(act(d, 'delete').plan(rows, {})[0]?.cmd.type).toBe(`Delete${k}`)
    }
  })
})
