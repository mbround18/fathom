/**
 * 040 US1 (FR-001): Copy / Download for the object in the Inspector's YAML tab, clean for
 * re-apply by default. Works from the raw JSON the Inspector already read, so it needs no
 * extra request and no write access.
 */
import { useMemo, useState } from 'react'

import type { ObjectRef } from '@/resources'

import { exportFilename, renderManifest } from './clean'
import { EXPORTABLE_KINDS } from './kinds'
import { ManifestActions } from './ManifestActions'

export function InspectorYamlExportBar({
  target,
  raw,
}: {
  target: ObjectRef
  raw: string | undefined
}) {
  const [clean, setClean] = useState(true)
  const rendered = useMemo(() => {
    if (raw === undefined) return undefined
    try {
      return renderManifest(raw, { clean })
    } catch {
      return undefined
    }
  }, [raw, clean])
  const kind = EXPORTABLE_KINDS[target.kind]?.kind ?? target.kind
  return (
    <div className="sticky top-0 z-10 border-b border-border bg-card px-2 py-0.5">
      <ManifestActions
        text={rendered?.yaml}
        filename={exportFilename(kind, [
          { namespace: target.namespace ?? null, name: target.name },
        ])}
        clean={{ value: clean, onChange: setClean }}
      />
      {rendered?.warnings.map((w) => (
        <p
          key={w}
          data-testid="manifest-warning"
          className="px-1 pb-1 font-mono text-xs text-amber-700 dark:text-amber-400"
        >
          {w}
        </p>
      ))}
    </div>
  )
}
