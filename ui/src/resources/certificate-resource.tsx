/**
 * cert-manager chain stage (spec 023): reached only by clicking a stage in the Certificates
 * pane's chain view, never listed in the navigator. Stage data comes from the last
 * `CertificateChainUpdated` in the store (no per-object backend read exists).
 */
import type { ChainStage } from '@/protocol/types'
import { useStore } from '@/store'

import type { DetailSection, ResourceDescriptor } from './types'

const SECTION = 'integrations/certificate-chain'

function currentStage(): ChainStage | undefined {
  const s = useStore.getState()
  const target = Object.values(s.panes).find(
    (p) => p.kind === 'inspector' && p.target?.section === SECTION,
  )?.target
  if (!target) return undefined
  return s.certManager.chain?.stages.find(
    (st) => st.kind === target.kind && st.name === target.name,
  )
}

const sections: DetailSection<ChainStage, ChainStage>[] = [
  {
    id: 'details',
    title: 'Details',
    render: ({ row, detail }) => {
      const d = detail ?? row ?? currentStage()
      if (!d) return null
      return {
        type: 'fields',
        fields: [
          { label: 'Kind', value: d.kind, mono: true },
          { label: 'Name', value: `${d.namespace}/${d.name}`, mono: true },
          { label: 'State', value: d.state, mono: true },
          { label: 'Reason', value: d.reason || '—' },
          { label: 'Message', value: d.message || '—' },
        ],
      }
    },
  },
]

export const certificateResource: ResourceDescriptor<ChainStage, ChainStage> = {
  section: SECTION,
  kind: 'CertificateChainStage',
  label: 'Certificate Chain Stage',
  namespaced: true,
  columns: [],
  rowKey: (r) => `${r.namespace}/${r.name}`,
  ref: (r) => ({ section: SECTION, kind: r.kind, namespace: r.namespace, name: r.name }),
  refresh: () => ({ type: 'RefreshCertManager' }),
  read: () => [],
  detailSections: sections,
  actions: [],
  selectListEvent: () => undefined,
  selectDetail: () => currentStage(),
}
