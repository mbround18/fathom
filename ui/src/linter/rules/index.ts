import type { Rule } from '../types'
import { deprecatedApi } from './deprecatedApi'
import { missingPdb } from './missingPdb'
import { nakedPod } from './nakedPod'
import { probes } from './probes'
import { rbacWildcard } from './rbacWildcard'
import { resourceLimits } from './resourceLimits'
import { unusedConfig } from './unusedConfig'
import { unusedServiceAccount } from './unusedServiceAccount'

/** The fixed v1 rule set (FR-004, FR-012). */
export const RULES: readonly Rule[] = [
  resourceLimits,
  probes,
  deprecatedApi,
  unusedConfig,
  unusedServiceAccount,
  nakedPod,
  rbacWildcard,
  missingPdb,
]
