import { useMemo } from 'react'

import { useStore } from '@/store'

import { Picker } from './Picker'

const ALL = '__all__'

/** A DNS-1123 label: what a namespace name may be. */
const NAMESPACE_NAME = /^[a-z0-9]([-a-z0-9]{0,61}[a-z0-9])?$/
const CUSTOM = { validate: (v: string) => NAMESPACE_NAME.test(v) }

/**
 * Namespace filter (T064); `all` sends an empty filter. Applies to every namespaced list.
 * 031: when the identity cannot list namespaces, the list holds only those it reaches and a
 * namespace can be typed in.
 */
export function NamespacePicker({ className }: { className?: string }) {
  const namespaces = useStore((s) => s.session.namespaces)
  const active = useStore((s) => s.session.activeNamespace)
  const selectNamespace = useStore((s) => s.selectNamespace)
  const restricted = useStore((s) => s.session.namespacesRestricted === true)
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
      {...(restricted ? { allowCustom: CUSTOM } : {})}
    />
  )
}
