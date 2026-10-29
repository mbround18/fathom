/**
 * A service's effective update policy, where it comes from, and every Watchtower label Depths saw
 * on it with what Depths did with it (spec 062 US4, T071/T074). Shared by the Updates page and the
 * stack page.
 */
import type { ServiceUpdate } from '@/protocol/generated'

import { labelEffectText, labelName, POLICY_LABEL, sourceText } from './model'

export function PolicyLine({ s }: { s: ServiceUpdate }) {
  const e = s.effective
  return (
    <span data-field="policy" data-policy={e.policy} className="text-muted-foreground">
      <span className="text-foreground">{POLICY_LABEL[e.policy]}</span> · {sourceText(e.source)}
    </span>
  )
}

export function LabelList({ s }: { s: ServiceUpdate }) {
  const labels = s.effective.labels
  if (labels.length === 0) return null
  return (
    <ul data-field="labels" aria-label="Watchtower labels" className="col-span-full flex min-w-0 basis-full flex-col gap-0.5">
      {labels.map((l) => (
        <li key={l.key} data-label={labelName(l.key)} data-effect={l.effect.kind} className="min-w-0 break-words">
          <code className="font-mono text-foreground">
            {labelName(l.key)}={l.value}
          </code>
          <span className={l.effect.kind === 'applied' ? ' text-muted-foreground' : ' text-status-warn'}>
            {' '}
            {labelEffectText(l.effect)}
          </span>
        </li>
      ))}
    </ul>
  )
}
