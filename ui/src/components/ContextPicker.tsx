import { useMemo } from 'react'

import { useStore } from '@/store'

import { Picker } from './Picker'

/** Kubeconfig context switcher (T064). Switching resets every list (FR-016). */
export function ContextPicker({ className }: { className?: string }) {
  const contexts = useStore((s) => s.session.contexts)
  const active = useStore((s) => s.session.activeContext)
  const selectContext = useStore((s) => s.selectContext)
  const options = useMemo(() => contexts.map((c) => ({ value: c, label: c })), [contexts])
  return (
    <Picker
      label="Context"
      prefix="ctx"
      value={active}
      options={options}
      placeholder="none"
      disabled={contexts.length === 0}
      onChange={selectContext}
      className={className}
    />
  )
}
