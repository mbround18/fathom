import { describe, expect, it } from 'vitest'

import { capabilityForSection } from '@/lib/capabilities'

describe('capabilityForSection for non-core kinds', () => {
  it.each([
    ['network/ingress', 'ingresses', 'networking.k8s.io'],
    ['network/networkpolicies', 'networkpolicies', 'networking.k8s.io'],
    ['storage/storageclasses', 'storageclasses', 'storage.k8s.io'],
    ['config/horizontalpodautoscalers', 'horizontalpodautoscalers', 'autoscaling'],
    ['config/verticalpodautoscalers', 'verticalpodautoscalers', 'autoscaling.k8s.io'],
    ['config/poddisruptionbudgets', 'poddisruptionbudgets', 'policy'],
    ['config/leases', 'leases', 'coordination.k8s.io'],
    ['config/mutatingwebhooks', 'mutatingwebhookconfigurations', 'admissionregistration.k8s.io'],
    ['network/endpointslices', 'endpointslices', 'discovery.k8s.io'],
    ['storage/csidrivers', 'csidrivers', 'storage.k8s.io'],
    ['storage/volumeattachments', 'volumeattachments', 'storage.k8s.io'],
    ['cluster/certificatesigningrequests', 'certificatesigningrequests', 'certificates.k8s.io'],
    ['workloads/controllerrevisions', 'controllerrevisions', 'apps'],
  ])('%s asks for %s in %s', (section, resource, apiGroup) => {
    expect(capabilityForSection(section)).toEqual({ resource, apiGroup })
  })

  it('leaves core serviceaccounts ungrouped', () => {
    expect(capabilityForSection('access/serviceaccounts')).toEqual({ resource: 'serviceaccounts' })
  })

  it('leaves core kinds without a group', () => {
    expect(capabilityForSection('config/configmaps')).toEqual({ resource: 'configmaps' })
  })
})
