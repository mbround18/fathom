import { endpoints } from './endpoints'
import { ingresses } from './ingress'
import { ingressClasses } from './ingressclasses'
import { networkPolicies } from './networkpolicies'
import type { AnyDescriptor } from './types'

/** Batch B (network): append descriptors here; index.ts already spreads this array. */
export const batchBDescriptors: AnyDescriptor[] = [
  endpoints,
  ingresses,
  ingressClasses,
  networkPolicies,
]
