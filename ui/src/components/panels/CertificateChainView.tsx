/** Vertical Certificate -> CertificateRequest -> Order -> Challenge chain, stuck stage highlighted. */
import type { CertificateChain } from '@/protocol/types'
import { useStore } from '@/store'
import { cn } from '@/lib/utils'

export function CertificateChainView({ chain }: { chain: CertificateChain }) {
  const openInspector = useStore((s) => s.openInspector)
  if (chain.not_found) {
    return (
      <p data-testid="cert-chain-not-found" className="text-xs text-muted-foreground">
        This certificate no longer exists.
      </p>
    )
  }
  return (
    <ol data-testid="cert-chain" className="flex flex-col gap-1">
      {chain.stages.map((st) => (
        <li key={`${st.kind}/${st.name}`}>
          <button
            type="button"
            data-testid="cert-chain-stage"
            data-stage={st.kind}
            data-stuck={st.is_stuck ? 'true' : 'false'}
            onClick={() =>
              openInspector(
                {
                  section: 'integrations/certificate-chain',
                  kind: st.kind,
                  namespace: st.namespace,
                  name: st.name,
                },
                undefined,
                true,
              )
            }
            className={cn(
              'w-full rounded-sm border px-2 py-1.5 text-left',
              st.is_stuck ? 'border-status-err bg-status-err/10' : 'border-border/60 bg-card/60',
            )}
          >
            <div className="font-mono text-xs">
              {st.kind} {st.name} <span className="text-muted-foreground">({st.state})</span>
              {st.is_stuck && <span className="ml-2 text-status-err">STUCK</span>}
            </div>
            {st.is_stuck && (st.reason || st.message) && (
              <div className="text-xs text-muted-foreground">
                {[st.reason, st.message].filter(Boolean).join(': ')}
              </div>
            )}
          </button>
        </li>
      ))}
    </ol>
  )
}
