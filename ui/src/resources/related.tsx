/**
 * Shared rendering for `RelatedResourceRef` (spec 004, User Story 3): every inspector with a
 * defined relationship renders its related resources through this one clickable-link component,
 * so `openInspector`'s push-vs-replace history behavior (`ui/src/store/index.ts`) is triggered
 * consistently everywhere instead of each kind reinventing it.
 */
import { StatusBadge } from '@/components/StatusBadge'
import type { RelatedResourceRef } from '@/protocol/types'
import { useStore } from '@/store'

import type { DetailBlock, ObjectRef } from './types'

export function relatedObjectRef(r: RelatedResourceRef): ObjectRef {
  return { section: r.section, kind: r.kind, namespace: r.namespace, name: r.name }
}

/** One related-resource entry: click opens it in the inspector and pushes onto the back-stack. */
export function RelatedResourceLink({ r }: { r: RelatedResourceRef }) {
  const openInspector = useStore((s) => s.openInspector)
  return (
    <button
      type="button"
      onClick={() => {
        openInspector(relatedObjectRef(r), undefined, true)
      }}
      className="flex min-w-0 items-center gap-2 text-left hover:text-accent-neon"
    >
      <span className="truncate font-mono">
        {r.namespace ? `${r.namespace}/` : ''}
        {r.name}
      </span>
      <span className="shrink-0 text-muted-foreground">{r.kind}</span>
      <span className="shrink-0 text-muted-foreground">{r.relation}</span>
      {r.status_hint && <StatusBadge status={r.status_hint} />}
    </button>
  )
}

/** A `DetailSection`'s `render` for a `related: RelatedResourceRef[]` list; `undefined` (not yet
 * fetched) renders the loading skeleton, an empty array renders "none". */
export function relatedResourcesBlock(
  related: RelatedResourceRef[] | undefined,
): DetailBlock | null {
  if (!related) return null
  return {
    type: 'list',
    items: related.map((r, i) => (
      <RelatedResourceLink key={`${r.kind}-${r.namespace ?? ''}-${r.name}-${String(i)}`} r={r} />
    )),
  }
}
