/** 034-simultaneous-federation: per-context switch adding/removing it from the live set. The
 * switch changes only what Fathom watches; it never mutates a cluster, so it is not `<Gated>`
 * (which checks the active context's identity, not the target's). Per-metric RBAC is reported
 * by the backend as `restricted` and rendered as "no access" by the row. */
import { Switch } from '@companion/ui'

export function FederationToggle({
  context,
  federated,
  onChange,
}: {
  context: string
  federated: boolean
  onChange: (context: string, on: boolean) => void
}) {
  return (
    <Switch
      data-testid="federation-toggle"
      aria-label={`Keep ${context} live`}
      checked={federated}
      onCheckedChange={(on) => onChange(context, on)}
    />
  )
}
