/**
 * Resource descriptors (data-model.md §5): one per kind, pure data plus small render functions.
 * `ResourceTable`, `Inspector`, and `NavTree` are driven entirely by these.
 *
 * Members are declared with method syntax on purpose: method parameters are bivariant, which
 * lets a `ResourceDescriptor<PodSummary, PodDetails>` live in the registry as an
 * `AnyDescriptor` without `any`.
 */
import type { ReactNode } from 'react'

import type {
  BackendCmd,
  RelatedResourceRef,
  ResourceListEvent,
  Route,
  SectionKey,
} from '@/protocol/types'
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
  /**
   * `RefreshRelatedResources`'s reply for this object (US3), when the descriptor's `read()`
   * requested one. `undefined` until it arrives (or if this kind never requests one).
   */
  related?: RelatedResourceRef[]
}

export interface DetailSection<R, D = unknown> {
  id: string
  title: string
  /** `null` = nothing to show yet (the detail is still loading). */
  render(ctx: DetailContext<R, D>): DetailBlock | null
}

/** What an action's dialog collected (the scale dialog's replica count). */
export interface ActionInput {
  replicas?: number
  /** Debug dialog (025): image and the container whose process namespace is shared. */
  image?: string
  targetContainer?: string
}

/** One command an action sends, and the rows it covers (for per-object error reporting). */
export interface ActionStep<R> {
  cmd: BackendCmd
  rows: R[]
}

/**
 * A mutating action (US3). Every one goes through a dialog that names each object
 * (constitution I): `confirm` → `ConfirmDialog`, `scale` → `ScaleDialog`.
 */
export interface ActionDescriptor<R> {
  id: 'delete' | 'scale' | 'restart' | 'debug'
  /** Menu and button label, e.g. `Delete`. */
  label: string
  /** Past tense for the result toast, e.g. `Deleted`. */
  done: string
  /** Present participle for the in-flight toast, e.g. `Deleting`. */
  doing: string
  prompt: 'confirm' | 'scale' | 'debug'
  destructive?: boolean
  /** Allowed on more than one row at once. */
  bulk?: boolean
  /** Scale: the current replica count shown as `current → new`. */
  current?(row: R): number
  /**
   * 018-rbac-aware-mutation-ui: the `SelfSubjectAccessReview` question this action depends on for
   * a single row. When present and the answer is a definite "no", the menu item is disabled with
   * the API server's reason; unknown answers leave it enabled (FR-006). Absent = ungated.
   */
  accessCheck?(row: R): { verb: string; resource: string; group?: string; namespace?: string | null }
  /** The commands to send for `rows` (one per row, or one `BulkDelete`). */
  plan(rows: R[], input: ActionInput): ActionStep<R>[]
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
