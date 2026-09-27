/**
 * Command palette (016-command-palette): pure functions that build and rank the palette's
 * candidate list from data already present in the store, without any new fetch or store slice
 * (data-model.md, research.md "no new index" decision).
 *
 * Resource results are derived on demand from `StoreState.lists` — the store never keeps a flat
 * "all resources" index, so this module reconstructs one, scoped to the active kubeconfig
 * context, each time the palette needs it. Namespace and section results are drawn from
 * `StoreState.session.namespaces` and `NAV_GROUPS` respectively.
 */
import type { SectionKey, ToolKind } from '@/protocol/types'
import type { ListKey, ListState, StoreState } from '@/store'
import { NO_CONTEXT } from '@/store/keys'

import { descriptorFor } from './index'
import {
  entryVisible,
  integrationVisible,
  NAV_GROUPS,
  navEntry,
  VELERO_SECTION,
  type NavVisibilityContext,
} from './nav'
import type { ObjectRef } from './types'

/** The operator's raw input, parsed into an optional `kind:` scope and the name to match. */
export interface PaletteQuery {
  raw: string
  /** Text before the first `:`, or `null` if the input has no `:`. */
  kind: string | null
  /** Text after the first `:` (may be empty), or the full `raw` string when `kind` is `null`. */
  name: string
}

export type PaletteResultCategory = 'resource' | 'namespace' | 'section' | 'tool'

export interface PaletteResourcePayload {
  ref: ObjectRef
}
export interface PaletteNamespacePayload {
  namespace: string
}
export interface PaletteSectionPayload {
  section: SectionKey
}

/** A tool pane, or the fleet dashboard (not a pane). */
export interface PaletteToolPayload {
  tool: ToolKind | 'fleet'
}

export interface PaletteResult {
  id: string
  category: PaletteResultCategory
  label: string
  sublabel?: string
  /** One-line plain-language description (043 FR-021: the navigator entry's description). */
  description?: string
  /** Higher = stronger match. Set by `rankAndFilter`; candidates start at `0`. */
  matchScore: number
  payload:
    PaletteResourcePayload | PaletteNamespacePayload | PaletteSectionPayload | PaletteToolPayload
}

/** Default cap on total results shown, per category, before ranking (FR-007). */
export const DEFAULT_RESULT_LIMIT = 8

/**
 * Split on the *first* `:` only, so a name that itself contains `:` after the prefix still
 * matches as free text (data-model.md's parsing rule).
 */
export function parsePaletteQuery(raw: string): PaletteQuery {
  const idx = raw.indexOf(':')
  if (idx === -1) return { raw, kind: null, name: raw }
  return { raw, kind: raw.slice(0, idx), name: raw.slice(idx + 1) }
}

/** Case-insensitive subsequence match — tolerant substring matching (research.md). */
function fuzzyMatch(query: string, text: string): boolean {
  if (query.length === 0) return true
  const q = query.toLowerCase()
  const t = text.toLowerCase()
  if (t.includes(q)) return true
  let qi = 0
  for (let ti = 0; ti < t.length && qi < q.length; ti++) {
    if (t[ti] === q[qi]) qi++
  }
  return qi === q.length
}

/**
 * Score a match: exact match highest, then prefix match, then substring, then subsequence.
 * Shorter `text` (a tighter match for the same query) scores slightly higher within a tier.
 */
function scoreMatch(query: string, text: string): number {
  if (query.length === 0) return 1
  const q = query.toLowerCase()
  const t = text.toLowerCase()
  const lengthBonus = 1 / (1 + t.length)
  if (t === q) return 1000 + lengthBonus
  if (t.startsWith(q)) return 800 + lengthBonus
  if (t.includes(q)) return 600 + lengthBonus
  return 400 + lengthBonus
}

/**
 * Build resource candidates from `store.lists`, scoped to the active kubeconfig context
 * (FR-012). Reads only `descriptor.ref(row)` plus the descriptor's own `kind`/`label` — never
 * row content beyond that (FR-013: no label/annotation/spec/status matching).
 */
export function buildResourceCandidates(
  lists: Record<ListKey, ListState>,
  activeContext: string | undefined,
): PaletteResult[] {
  const contextSegment = activeContext ?? NO_CONTEXT
  const out: PaletteResult[] = []
  for (const [key, list] of Object.entries(lists)) {
    const parts = key.split('|')
    const keyContext = parts[0]
    const sectionPart = parts[1]
    if (keyContext !== contextSegment || sectionPart === undefined) continue
    const section: SectionKey = sectionPart
    const descriptor = descriptorFor(section)
    if (!descriptor) continue
    for (const row of list.rows) {
      const ref = descriptor.ref(row)
      out.push({
        id: `resource:${ref.section}:${ref.namespace ?? ''}:${ref.name}`,
        category: 'resource',
        label: ref.name,
        sublabel: `${descriptor.kind} · ${ref.namespace ?? '—'}`,
        matchScore: 0,
        payload: { ref },
      })
    }
  }
  return out
}

/** Namespace candidates from the store's already-loaded namespace list (FR-016). */
export function buildNamespaceCandidates(namespaces: readonly string[]): PaletteResult[] {
  return namespaces.map((namespace) => ({
    id: `namespace:${namespace}`,
    category: 'namespace',
    label: namespace,
    matchScore: 0,
    payload: { namespace },
  }))
}

/**
 * Section candidates from `NAV_GROUPS` — static data, always available regardless of connection
 * state (research.md's decision to scope pane search to `NAV_GROUPS`, not `ToolKind` dock tabs).
 * "More" (advanced) entries are indexed too (043 FR-022). With `ctx`, entries the navigator hides
 * for this cluster (Velero until detected, FR-036) are left out as well.
 */
export function buildSectionCandidates(ctx?: NavVisibilityContext): PaletteResult[] {
  const out: PaletteResult[] = []
  for (const group of NAV_GROUPS) {
    for (const section of group.sections) {
      const model = navEntry(section.key)
      if (ctx && model && !entryVisible(model, ctx)) continue
      out.push({
        id: `section:${section.key}`,
        category: 'section',
        label: section.label,
        sublabel: group.label,
        description: section.description,
        matchScore: 0,
        payload: { section: section.key },
      })
    }
  }
  return out
}

/** Tools and dashboards that have no navigator section (032-ui-shell-a11y). */
const TOOL_ENTRIES: readonly { tool: ToolKind | 'fleet'; label: string }[] = [
  { tool: 'fleet', label: 'Fleet dashboard' },
  { tool: 'logs', label: 'Logs' },
  { tool: 'events', label: 'Events tool' },
  { tool: 'portforwards', label: 'Port forwards tool' },
  { tool: 'topology', label: 'Topology' },
  { tool: 'knative', label: 'Knative' },
  { tool: 'keda', label: 'KEDA autoscalers' },
  { tool: 'linter', label: 'Linter' },
  { tool: 'velero', label: 'Velero backups' },
  { tool: 'certManager', label: 'Certificates (cert-manager)' },
  { tool: 'vulnerabilities', label: 'Trivy vulnerabilities' },
]

/** Tool candidates; with `ctx`, Velero only once it is detected (043 FR-036). */
export function buildToolCandidates(
  ctx?: Pick<NavVisibilityContext, 'installed'>,
): PaletteResult[] {
  return TOOL_ENTRIES.filter(
    ({ tool }) =>
      !ctx ||
      tool !== 'velero' ||
      integrationVisible(VELERO_SECTION, ctx.installed(VELERO_SECTION)),
  ).map(({ tool, label }) => ({
    id: `tool:${tool}`,
    category: 'tool',
    label,
    sublabel: tool === 'fleet' ? 'Dashboard' : 'Tool',
    matchScore: 0,
    payload: { tool },
  }))
}

/**
 * Convenience: build every category's candidates from the store snapshot in one call.
 */
export function buildAllCandidates(store: {
  lists: Record<ListKey, ListState>
  session: { activeContext?: string; namespaces: readonly string[] }
}): PaletteResult[] {
  return [
    ...buildResourceCandidates(store.lists, store.session.activeContext),
    ...buildNamespaceCandidates(store.session.namespaces),
    ...buildSectionCandidates(),
    ...buildToolCandidates(),
  ]
}

/**
 * Apply `query` to `candidates`: when `query.kind` is set, first keep only resource candidates
 * whose descriptor kind fuzzily matches it (an unrecognized kind yields zero resource results —
 * not an unfiltered fallback, per the spec's edge case); non-resource candidates are unaffected
 * by the `kind` scope. Then fuzzy-match `query.name` against each candidate's `label`, drop
 * non-matches, sort by `matchScore` descending, and cap to `limit` per category.
 */
export function rankAndFilter(
  candidates: readonly PaletteResult[],
  query: PaletteQuery,
  limit: number = DEFAULT_RESULT_LIMIT,
): PaletteResult[] {
  const byCategory = new Map<PaletteResultCategory, PaletteResult[]>()
  for (const candidate of candidates) {
    if (query.kind && candidate.category === 'resource') {
      const ref = (candidate.payload as PaletteResourcePayload).ref
      const descriptor = descriptorFor(ref.section)
      const kindText = descriptor?.kind ?? ref.kind
      if (!fuzzyMatch(query.kind, kindText)) continue
    }
    if (!fuzzyMatch(query.name, candidate.label)) continue
    const scored: PaletteResult = {
      ...candidate,
      matchScore: scoreMatch(query.name, candidate.label),
    }
    const bucket = byCategory.get(candidate.category)
    if (bucket) bucket.push(scored)
    else byCategory.set(candidate.category, [scored])
  }

  const out: PaletteResult[] = []
  for (const bucket of byCategory.values()) {
    bucket.sort((a, b) => b.matchScore - a.matchScore)
    out.push(...bucket.slice(0, limit))
  }
  return out
}

/** Narrow helper so calling code can avoid importing `StoreState` just for its shape. */
export type PaletteStoreSnapshot = Pick<StoreState, 'lists' | 'session'>
