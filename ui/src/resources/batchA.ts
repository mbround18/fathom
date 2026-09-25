import { cronjobs } from './cronjobs'
import { jobs } from './jobs'
import { replicasets } from './replicasets'
import { replicationcontrollers } from './replicationcontrollers'
import type { AnyDescriptor } from './types'

/** Batch A (workloads: jobs, cronjobs, replicasets, RCs); index.ts spreads this array. */
export const batchADescriptors: AnyDescriptor[] = [
  jobs,
  cronjobs,
  replicasets,
  replicationcontrollers,
]
