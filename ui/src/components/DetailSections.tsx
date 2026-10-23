/**
 * Renders a descriptor's detail sections (T063). Descriptors return data (`DetailBlock`); this
 * owns the look. Every section always renders its heading, with a skeleton until data arrives.
 */
import type { ReactNode } from 'react'

import { InspectorMetrics, METRICS_KINDS } from '@/components/InspectorMetrics'
import { Skeleton } from '@/components/ui/skeleton'
import { cn } from '@companion/theme'
import type { RelatedResourceRef } from '@/protocol/types'
import type { AnyDescriptor, DetailBlock } from '@/resources'
import type { ListRow } from '@/store'

interface DetailSectionsProps {
  descriptor: AnyDescriptor
  row?: ListRow
  detail?: unknown
  /** Column ids the source list is hiding for lack of width (FR-009): shown here instead. */
  hiddenColumns?: readonly string[]
  /** US3: `RefreshRelatedResources`'s reply for the current target, if it requested one. */
  related?: RelatedResourceRef[]
  /** T105: when set (Node/Pod/workload), a Metrics section is appended. */
  metricsTarget?: { kind: string; namespace?: string | null | undefined; name: string }
}

export function DetailSections({
  descriptor,
  row,
  detail,
  hiddenColumns,
  related,
  metricsTarget,
}: DetailSectionsProps) {
  const extra =
    row && hiddenColumns && hiddenColumns.length > 0
      ? descriptor.columns.filter((c) => hiddenColumns.includes(c.id))
      : []
  return (
    <div
      data-testid="detail-sections"
      data-section={descriptor.section}
      className="flex flex-col gap-5 p-3"
    >
      {descriptor.detailSections.map((s) => (
        <Section key={s.id} title={s.title}>
          <Block block={s.render({ row, detail, related })} />
        </Section>
      ))}
      {metricsTarget && METRICS_KINDS.has(metricsTarget.kind) && (
        <Section title="Metrics">
          <InspectorMetrics target={metricsTarget} />
        </Section>
      )}
      {extra.length > 0 && row && (
        <Section title="More columns">
          <Block
            block={{
              type: 'fields',
              fields: extra.map((c) => ({ label: c.header, value: c.cell(row), mono: !c.status })),
            }}
          />
        </Section>
      )}
    </div>
  )
}

function Section({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section className="flex min-w-0 flex-col gap-2">
      <h3 className="flex items-center gap-2 font-display text-xs font-semibold tracking-[0.2em] text-accent-neon uppercase">
        <span>{title}</span>
        <span
          aria-hidden="true"
          className="h-px flex-1 bg-gradient-to-r from-border to-transparent"
        />
      </h3>
      {children}
    </section>
  )
}

function Block({ block }: { block: DetailBlock | null }) {
  if (!block) {
    return (
      <div role="status" aria-label="Loading" className="flex flex-col gap-1.5">
        <Skeleton className="h-3 w-3/4" />
        <Skeleton className="h-3 w-1/2" />
      </div>
    )
  }
  switch (block.type) {
    case 'fields':
      return (
        <dl className="grid grid-cols-[minmax(6rem,max-content)_minmax(0,1fr)] gap-x-4 gap-y-1 text-sm">
          {block.fields.map((f, i) => (
            <div key={`${f.label}-${String(i)}`} className="contents">
              <dt className="text-muted-foreground">{f.label}</dt>
              <dd className={cn('min-w-0 break-words', f.mono && 'font-mono text-[0.85em]')}>
                {f.value}
              </dd>
            </div>
          ))}
        </dl>
      )
    case 'map': {
      const entries = Object.entries(block.entries)
      if (entries.length === 0) return <p className="text-sm text-muted-foreground">none</p>
      return (
        <ul className="flex flex-wrap gap-1.5">
          {entries.map(([k, v]) => (
            <li
              key={k}
              className="max-w-full border border-border bg-muted/40 px-1.5 py-0.5 font-mono text-xs break-all"
            >
              <span className="text-muted-foreground">{k}</span>
              <span aria-hidden="true" className="text-accent-neon">
                =
              </span>
              <span className="sr-only"> equals </span>
              {v}
            </li>
          ))}
        </ul>
      )
    }
    case 'list':
      if (block.items.length === 0) return <p className="text-sm text-muted-foreground">none</p>
      return (
        <ul className="flex flex-col gap-1 font-mono text-xs">
          {block.items.map((item, i) => (
            <li key={i} className="min-w-0 border-l border-accent-neon/50 pl-2 break-words">
              {item}
            </li>
          ))}
        </ul>
      )
    case 'text':
      return <p className="text-sm whitespace-pre-wrap">{block.text}</p>
  }
}
