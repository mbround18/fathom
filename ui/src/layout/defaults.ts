/**
 * Default layout profile per width class, and validation of saved profiles
 * (contracts/layout.md § Slot profiles, FR-006/FR-007).
 */
import type { DockConfig, LayoutProfile, SlotConfig, ToolKind } from '@/protocol/types'

import { hasDock, visibleSlots, type WidthClass } from './width-class'

const DOCK: DockConfig = {
  open: true,
  heightPct: 30,
  tabs: ['logs', 'events', 'topology', 'velero', 'certManager', 'knative'],
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
  wide: [
    { kind: 'nav', sizePct: 12 },
    { kind: 'list', sizePct: 30 },
    { kind: 'list', sizePct: 30 },
    { kind: 'inspector', sizePct: 28 },
  ],
  ultra: [
    { kind: 'nav', sizePct: 10 },
    { kind: 'list', sizePct: 24 },
    { kind: 'list', sizePct: 24 },
    { kind: 'inspector', sizePct: 22 },
    { kind: 'tool', sizePct: 20, tool: 'overview' },
  ],
}

/** The 6th slot at ≥ 3840 px. */
const ULTRA_PLUS_SLOT: SlotConfig = { kind: 'tool', sizePct: 16, tool: 'flux' }

const SLOT_KINDS: ReadonlySet<string> = new Set(['nav', 'list', 'inspector', 'tool'])
const TOOL_KINDS: ReadonlySet<string> = new Set<ToolKind>([
  'overview',
  'metrics',
  'flux',
  'hubble',
  'vulnerabilities',
  'logs',
  'events',
  'portforwards',
  'topology',
  'velero',
  'knative',
  'certManager',
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

/**
 * The profile to render for `cls`: the saved one when it is well formed, else the default.
 * Saved slots beyond `visibleSlots` are truncated (their panes survive in the store; only the
 * slot is hidden) and sizes renormalized. The dock is dropped below desktop.
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
  const slots = normalize(saved.slots.slice(0, max).map((s) => ({ ...s })))
  if (!hasDock(cls)) return { slots }
  const dock = isValidDock(saved.dock)
    ? { ...saved.dock, tabs: [...saved.dock.tabs] }
    : fallback.dock
  return dock ? { slots, dock } : { slots }
}
