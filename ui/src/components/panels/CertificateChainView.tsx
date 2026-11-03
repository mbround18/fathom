/** Vertical Certificate -> CertificateRequest -> Order -> Challenge chain, stuck stage highlighted. */
import type { CertificateChain } from '@/protocol/types'
import { useStore } from '@/store'
import { StageList, type Stage } from '@companion/ui/kit'

export function CertificateChainView({ chain }: { chain: CertificateChain }) {
  const openInspector = useStore((s) => s.openInspector)
  if (chain.not_found) {
    return (
      <p data-testid="cert-chain-not-found" className="text-xs text-muted-foreground">
        This certificate no longer exists.
      </p>
    )
  }
  // Only the stuck stage is marked: the chain says which link is stuck, not that the others are
  // done (a Certificate before a stuck Challenge is not Ready).
  const stages: Stage[] = chain.stages.map((st) => {
    const note = [st.reason, st.message].filter(Boolean).join(': ')
    return {
      key: st.kind,
      title: `${st.kind} ${st.name}`,
      status: st.state,
      state: st.is_stuck ? 'stuck' : undefined,
      ...(st.is_stuck && note ? { note } : {}),
      onOpen: () => {
        openInspector(
          { section: 'integrations/certificate-chain', kind: st.kind, namespace: st.namespace, name: st.name },
          undefined,
          true,
        )
      },
    }
  })
  return (
    <div data-testid="cert-chain">
      <StageList label="Certificate chain" stages={stages} />
    </div>
  )
}
