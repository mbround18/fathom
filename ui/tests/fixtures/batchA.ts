/** Batch A fixtures (jobs, cronjobs): rows for the e2e specs. */
import type { CronJobSummary, JobSummary } from '../../src/protocol/types'

export const JOBS: JobSummary[] = [
  {
    namespace: 'batch-ns',
    name: 'nightly-1',
    completions: '1/1',
    duration: '4m',
    active: 0,
    succeeded: 1,
    failed: 0,
    age: '2h',
  },
  {
    namespace: 'batch-ns',
    name: 'flaky-1',
    completions: '0/1',
    duration: '9m',
    active: 0,
    succeeded: 0,
    failed: 3,
    age: '1h',
  },
]

export const CRONJOBS: CronJobSummary[] = [
  {
    namespace: 'batch-ns',
    name: 'nightly',
    schedule: '0 2 * * *',
    suspend: false,
    active: 0,
    last_schedule: '2h',
    age: '9d',
  },
  {
    namespace: 'batch-ns',
    name: 'paused',
    schedule: '*/5 * * * *',
    suspend: true,
    active: 0,
    last_schedule: null,
    age: '3d',
  },
]
