import type { AnyDescriptor } from './types'

import {
  configMaps,
  limitRanges,
  podDisruptionBudgets,
  resourceQuotas,
  secrets,
} from './configKinds'
import { persistentVolumeClaims, persistentVolumes, storageClasses } from './storage'

/** Batch C (storage + config core): index.ts already spreads this array. */
export const batchCDescriptors: AnyDescriptor[] = [
  persistentVolumes,
  persistentVolumeClaims,
  storageClasses,
  configMaps,
  secrets,
  resourceQuotas,
  limitRanges,
  podDisruptionBudgets,
]
