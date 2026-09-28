/**
 * Default layout profile per width class, and validation of saved profiles
 * (contracts/layout.md § Slot profiles, FR-006/FR-007).
 */
import type { DockConfig, LayoutProfile, SlotConfig, ToolKind } from '@/protocol/types'

import { hasDock, visibleSlots, type WidthClass } from './width-class'

/**
 * 043: the dock holds only streams that follow what the operator is doing. Every other tool is
 * a destination in the navigator (Topology, Linter, the integrations). The `ToolKind` enum is
 * unchanged, so saved docks naming other tools stay valid and are filtered on load.
 */
export const DOCK_ALLOWED: readonly ToolKind[] = ['logs', 'events', 'daemon']
const DOCK_ALLOWED_SET: ReadonlySet<ToolKind> = new Set(DOCK_ALLOWED)

/** Default dock: Logs and Events. Cluster history (`daemon`) is added by `dockTabs`. */
export const DOCK: DockConfig = {
  open: true,
  heightPct: 30,
  tabs: ['logs', 'events'],
}

/** Drop tabs the dock no longer accepts, keeping their order, `open` and `heightPct` (FR-025). */
export function filterDock(dock: DockConfig): DockConfig {
  return { ...dock, tabs: dock.tabs.filter((t) => DOCK_ALLOWED_SET.has(t)) }
}

/**
 * The tabs to render: the filtered dock, with Cluster history present exactly while the daemon
 * is available for the active context (FR-024). Appended when missing, removed when unavailable.
 */
export function dockTabs(dock: DockConfig, daemonAvailable: boolean): ToolKind[] {
  const tabs = filterDock(dock).tabs.filter((t) => t !== 'daemon' || daemonAvailable)
  if (daemonAvailable && !tabs.includes('daemon')) tabs.push('daemon')
  return tabs
}

const PROFILES: Readonly<Record<WidthClass, readonly SlotConfig[]>> = {
  phone: [{ kind: 'list', sizePct: 100 }],
  tablet: [
    { kind: 'list', sizePct: 55 },
    { kind: 'inspector', sizePct: 45 },
  ],
  laptop: [
    { kind: 'nav', sizePct: 16 },
    { kind: 'list', sizePct: 50 },
    { kind: 'inspector', sizePct: 34 },
  ],
  desktop: [
    { kind: 'nav', sizePct: 16 },
    { kind: 'list', sizePct: 48 },
    { kind: 'inspector', sizePct: 36 },
  ],
  // 044 (FR-001/FR-002): navigator, the selected section, then cascade columns that stay empty
  // until the operator drills in. Slot counts and sizes are unchanged (Principle IV).
  wide: [
    { kind: 'nav', sizePct: 12 },
    { kind: 'list', sizePct: 30 },
    { kind: 'inspector', sizePct: 30 },
    { kind: 'inspector', sizePct: 28 },
  ],
  ultra: [
    { kind: 'nav', sizePct: 10 },
    { kind: 'list', sizePct: 24 },
    { kind: 'inspector', sizePct: 24 },
    { kind: 'inspector', sizePct: 22 },
    { kind: 'inspector', sizePct: 20 },
  ],
}

/** The 6th slot at ≥ 3840 px: one more cascade column (044). */
const ULTRA_PLUS_SLOT: SlotConfig = { kind: 'inspector', sizePct: 16 }

const SLOT_KINDS: ReadonlySet<string> = new Set(['nav', 'list', 'inspector', 'tool'])
const TOOL_KINDS: ReadonlySet<string> = new Set<ToolKind>([
  'overview',
  'metrics',
  'flux',
  'hubble',
  'vulnerabilities',
  'keda',
  'logs',
  'events',
  'portforwards',
  'topology',
  'daemon',
  'velero',
  'knative',
  'certManager',
  'linter',
])

/** Rescale sizePct so the slots sum to 100 (rounded to 0.01). */
function normalize(slots: SlotConfig[]): SlotConfig[] {
  const total = slots.reduce((sum, s) => sum + s.sizePct, 0)
  if (total <= 0) return slots.map((s) => ({ ...s, sizePct: 100 / slots.length }))
  return slots.map((s) => ({ ...s, sizePct: Math.round((s.sizePct / total) * 10000) / 100 }))
}

export function defaultProfile(cls: WidthClass, ultraPlus = false): LayoutProfile {
  const slots = PROFILES[cls].map((s) => ({ ...s }))
  if (cls === 'ultra' && ultraPlus) slots.push({ ...ULTRA_PLUS_SLOT })
  return {
    slots: normalize(slots),
    ...(hasDock(cls) ? { dock: { ...DOCK, tabs: [...DOCK.tabs] } } : {}),
  }
}

function isValidSlot(slot: unknown): slot is SlotConfig {
  if (typeof slot !== 'object' || slot === null) return false
  const s = slot as Partial<SlotConfig>
  return (
    typeof s.kind === 'string' &&
    SLOT_KINDS.has(s.kind) &&
    typeof s.sizePct === 'number' &&
    Number.isFinite(s.sizePct) &&
    s.sizePct > 0 &&
    (s.kind !== 'tool' || (typeof s.tool === 'string' && TOOL_KINDS.has(s.tool)))
  )
}

function isValidDock(dock: unknown): dock is DockConfig {
  if (typeof dock !== 'object' || dock === null) return false
  const d = dock as Partial<DockConfig>
  return (
    typeof d.open === 'boolean' &&
    typeof d.heightPct === 'number' &&
    d.heightPct > 0 &&
    d.heightPct < 100 &&
    Array.isArray(d.tabs) &&
    d.tabs.every((t) => typeof t === 'string' && TOOL_KINDS.has(t))
  )
}

/** The classes whose slot row is fixed by the cascade (044). */
const CASCADE_CLASSES: ReadonlySet<WidthClass> = new Set<WidthClass>(['wide', 'ultra'])

/**
 * 044 (FR-011): at wide and above the row is always navigator, section list, cascade columns. A
 * profile saved by an earlier build named a second list, Overview or Flux from index 2 on; only
 * the widths the operator dragged survive it, and `UiConfig.schema` stays 1 so the file still
 * loads on the previous build.
 */
function cascadeKinds(slots: readonly SlotConfig[]): SlotConfig[] {
  return slots.map((s, i) => ({
    kind: i === 0 ? 'nav' : i === 1 ? 'list' : 'inspector',
    sizePct: s.sizePct,
  }))
}

/**
 * The profile to render for `cls`: the saved one when it is well formed, else the default.
 * Saved slots beyond `visibleSlots` are truncated (their panes survive in the store; only the
 * slot is hidden) and sizes renormalized. The dock is dropped below laptop, and a saved dock
 * keeps only the tabs `filterDock` allows.
 */
export function resolveProfile(
  cls: WidthClass,
  saved?: LayoutProfile | null,
  ultraPlus = false,
): LayoutProfile {
  const fallback = defaultProfile(cls, ultraPlus)
  if (!saved || !Array.isArray(saved.slots) || saved.slots.length === 0) return fallback
  if (!saved.slots.every(isValidSlot)) return fallback

  const max = visibleSlots(cls, ultraPlus ? Infinity : 0)
  const truncated = saved.slots.slice(0, max).map((s) => ({ ...s }))
  const slots = normalize(CASCADE_CLASSES.has(cls) ? cascadeKinds(truncated) : truncated)
  if (!hasDock(cls)) return { slots }
  // Filtered in memory only: the saved file is rewritten on the user's next layout change.
  const dock = isValidDock(saved.dock) ? filterDock(saved.dock) : fallback.dock
  return dock ? { slots, dock } : { slots }
}
