import { useMemo } from 'react'

import { useStore } from '@/store'

import { Picker } from './Picker'

const ALL = '__all__'

/** Namespace filter (T064); `all` sends an empty filter. Applies to every namespaced list. */
export function NamespacePicker({ className }: { className?: string }) {
  const namespaces = useStore((s) => s.session.namespaces)
  const active = useStore((s) => s.session.activeNamespace)
  const selectNamespace = useStore((s) => s.selectNamespace)
  const options = useMemo(
    () => [{ value: ALL, label: 'all' }, ...namespaces.map((n) => ({ value: n, label: n }))],
    [namespaces],
  )
  return (
    <Picker
      label="Namespace"
      prefix="ns"
      value={active ?? ALL}
      options={options}
      onChange={(v) => selectNamespace(v === ALL ? null : v)}
      className={className}
    />
  )
}
