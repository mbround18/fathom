/**
 * Certificates (cert-manager) tool pane: renders `store.certManager.overview`
 * (`CertManagerOverviewUpdated`) with expiry countdowns, and a selected Certificate's lifecycle
 * chain (`CertificateChainUpdated`). Polled every 20s by the backend; opening also refreshes.
 */
import { CertificateIcon } from '@phosphor-icons/react'
import { useEffect, useState } from 'react'

import { formatExpiryCountdown } from '@/lib/cert-expiry'
import type { CertExpiryState, CertReadyState, CertificateSummary } from '@/protocol/types'
import { useStore } from '@/store'

import { CertificateChainView } from './CertificateChainView'
import { IntegrationSummary, type SummaryRow, type SummaryTile } from './IntegrationSummary'
import { MetricsUnavailable } from '../shell/PaneStates'

const READY_TONE: Record<CertReadyState, SummaryRow['tone']> = {
  Ready: 'ok',
  Renewing: 'warn',
  Failed: 'err',
  Pending: 'muted',
}
const EXPIRY_TONE: Record<CertExpiryState, SummaryRow['tone']> = {
  Valid: 'ok',
  ExpiringSoon: 'warn',
  Expired: 'err',
  Unknown: 'muted',
}

function urgency(c: CertificateSummary): number {
  if (c.ready === 'Failed' || c.expiry_state === 'Expired') return 0
  if (c.expiry_state === 'ExpiringSoon' || c.ready === 'Renewing' || c.ready === 'Pending') return 1
  return 2
}

export function CertManagerPanel() {
  const overview = useStore((s) => s.certManager.overview)
  const chain = useStore((s) => s.certManager.chain)
  const refreshCertManager = useStore((s) => s.refreshCertManager)
  const selectCertificateChain = useStore((s) => s.selectCertificateChain)
  const [selected, setSelected] = useState<{ namespace: string; name: string } | null>(null)

  useEffect(() => {
    refreshCertManager()
  }, [refreshCertManager])

  if (!overview) {
    return <MetricsUnavailable detail="Waiting for the cert-manager scan to complete." />
  }
  if (!overview.installed) {
    return <MetricsUnavailable detail="cert-manager is not installed in this cluster." />
  }
  if (overview.certificates.length === 0) {
    return (
      <MetricsUnavailable detail="cert-manager is installed, but no Certificates were found." />
    )
  }

  const certs = [...overview.certificates].sort((a, b) => urgency(a) - urgency(b))
  const tiles: SummaryTile[] = [
    { label: 'Certificates', value: String(certs.length), tone: 'ok' },
    {
      label: 'Expiring/Expired',
      value: String(
        certs.filter((c) => c.expiry_state === 'ExpiringSoon' || c.expiry_state === 'Expired')
          .length,
      ),
      tone: 'warn',
    },
    {
      label: 'Failed',
      value: String(certs.filter((c) => c.ready === 'Failed').length),
      tone: 'err',
    },
  ]
  const rows: SummaryRow[] = certs.map((c) => ({
    key: `${c.namespace}/${c.name}`,
    primary: (
      <button
        type="button"
        data-testid="cert-row"
        data-ready={c.ready}
        data-expiry={c.expiry_state}
        className="w-full truncate text-left"
        onClick={() => {
          setSelected({ namespace: c.namespace, name: c.name })
          selectCertificateChain(c.namespace, c.name)
        }}
      >
        {c.namespace}/{c.name} · {c.issuer_ref} · {c.ready} ·{' '}
        {formatExpiryCountdown(c.expiry_seconds)}
      </button>
    ),
    secondary: c.message || undefined,
    tone: c.ready === 'Ready' ? EXPIRY_TONE[c.expiry_state] : READY_TONE[c.ready],
  }))

  const showChain =
    selected &&
    chain &&
    chain.certificate_namespace === selected.namespace &&
    chain.certificate_name === selected.name

  return (
    <div className="flex h-full min-h-0 flex-col">
      <IntegrationSummary icon={CertificateIcon} title="Certificates" tiles={tiles} rows={rows} />
      {showChain && (
        <div className="border-t border-border p-3">
          <CertificateChainView chain={chain} />
        </div>
      )}
    </div>
  )
}
