import { describe, expect, it } from 'vitest'

import { capabilityForSection } from '@/lib/capabilities'

describe('capabilityForSection for non-core kinds', () => {
  it.each([
    ['network/ingress', 'ingresses', 'networking.k8s.io'],
    ['network/networkpolicies', 'networkpolicies', 'networking.k8s.io'],
    ['storage/storageclasses', 'storageclasses', 'storage.k8s.io'],
    ['config/horizontalpodautoscalers', 'horizontalpodautoscalers', 'autoscaling'],
    ['config/poddisruptionbudgets', 'poddisruptionbudgets', 'policy'],
    ['config/leases', 'leases', 'coordination.k8s.io'],
    ['config/mutatingwebhooks', 'mutatingwebhookconfigurations', 'admissionregistration.k8s.io'],
  ])('%s asks for %s in %s', (section, resource, apiGroup) => {
    expect(capabilityForSection(section)).toEqual({ resource, apiGroup })
  })

  it('leaves core kinds without a group', () => {
    expect(capabilityForSection('config/configmaps')).toEqual({ resource: 'configmaps' })
  })
})
