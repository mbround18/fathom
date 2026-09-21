/**
 * Resource descriptors (data-model.md §5): one per kind, pure data plus small render functions.
 * `ResourceTable`, `Inspector`, and `NavTree` are driven entirely by these.
 *
 * Members are declared with method syntax on purpose: method parameters are bivariant, which
 * lets a `ResourceDescriptor<PodSummary, PodDetails>` live in the registry as an
 * `AnyDescriptor` without `any`.
 */
import type { ReactNode } from 'react'

import type { BackendCmd, ResourceListEvent, Route, SectionKey } from '@/protocol/types'
import type { DetailState, ListRow } from '@/store'

/** Stable row identity within a list: `namespace/name` (or `name` when cluster-scoped). */
export type RowKey = string

/** Points at one object: what the inspector shows and `read` asks the backend for. */
export interface ObjectRef {
  section: SectionKey
  kind: string
  namespace: string | null
  name: string
}

/** 1 = always shown; higher numbers hide first as the pane narrows (contracts/layout.md). */
export type ColumnPriority = 1 | 2 | 3 | 4 | 5

export interface ColumnDef<R> {
  id: string
  header: string
  priority: ColumnPriority
  /** Narrowest width, px, at which the column is readable. Columns hide rather than squash. */
  minWidth: number
  /** Share of spare width (CSS grid `fr`); default 1. */
  grow?: number
  align?: 'start' | 'end'
  cell(row: R): ReactNode
  /** Plain text of the cell: `title` tooltips, the inspector summary, and tests. */
  text(row: R): string
  /** Ascending comparator; omitted = sort by `text`. */
  sort?(a: R, b: R): number
  /** Marks the status column (inverse-video rows recolour it). */
  status?: boolean
}

/** What a detail section renders: data only, so `DetailSections` owns the look. */
export type DetailBlock =
  | { type: 'fields'; fields: { label: string; value: ReactNode; mono?: boolean }[] }
  | { type: 'map'; entries: Record<string, string> }
  | { type: 'list'; items: ReactNode[] }
  | { type: 'text'; text: string }

export interface DetailContext<R, D> {
  /** The list row, when the object is still in its list. */
  row?: R
  /** The backend's read reply, once it has arrived. */
  detail?: D
}

export interface DetailSection<R, D = unknown> {
  id: string
  title: string
  /** `null` = nothing to show yet (the detail is still loading). */
  render(ctx: DetailContext<R, D>): DetailBlock | null
}

/** Actions (delete, scale, restart, logs, port-forward, …) arrive in US3. */
export interface ActionDescriptor<R> {
  id: string
  label: string
  destructive?: boolean
  bulk?: boolean
  command(rows: R[], ctx: { requestId: number }): BackendCmd | undefined
}

export interface ResourceDescriptor<R, D = unknown> {
  section: SectionKey
  /** Kubernetes kind, e.g. `Pod`; matches the store's detail kind (`PodDetailsUpdated` → `Pod`). */
  kind: string
  /** Plural label for titles and the navigator. */
  label: string
  namespaced: boolean
  columns: ColumnDef<R>[]
  rowKey(row: R): RowKey
  ref(row: R): ObjectRef
  refresh(route: Route): BackendCmd
  /** Detail + YAML (raw_json) + events. */
  read?(ref: ObjectRef): BackendCmd[]
  detailSections: DetailSection<R, D>[]
  actions: ActionDescriptor<R>[]
  selectListEvent(event: ResourceListEvent): R[] | undefined
  /** The typed read reply from the store's detail cache, if it is this kind's. */
  selectDetail(detail: DetailState): D | undefined
  /** Raw manifest JSON for the YAML tab. */
  rawJson?(detail: D): string
}

export type AnyDescriptor = ResourceDescriptor<ListRow>
