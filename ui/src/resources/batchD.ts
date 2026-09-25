import { horizontalpodautoscalers } from './horizontalpodautoscalers'
import { leases } from './leases'
import { priorityclasses } from './priorityclasses'
import { runtimeclasses } from './runtimeclasses'
import type { AnyDescriptor } from './types'
import { verticalpodautoscalers } from './verticalpodautoscalers'
import { mutatingwebhooks, validatingwebhooks } from './webhooks'

/** Batch D (cluster-scoped / policy config): append descriptors here; index.ts already spreads this array. */
export const batchDDescriptors: AnyDescriptor[] = [
  horizontalpodautoscalers,
  verticalpodautoscalers,
  priorityclasses,
  runtimeclasses,
  leases,
  mutatingwebhooks,
  validatingwebhooks,
]
