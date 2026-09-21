/**
 * Local stand-ins for protocol-derived types that will be generated from Rust by ts-rs into
 * `ui/src/protocol/generated/` (T011–T021). Shapes follow data-model.md §1–§2.
 *
 * TODO(protocol): replace with generated types. When `generated/index.ts` lands, delete this file
 * and re-point imports; anything that no longer type-checks is a real contract mismatch.
 */

// TODO(protocol): replace with generated types
export type WidthClass = 'phone' | 'tablet' | 'laptop' | 'desktop' | 'wide' | 'ultra'

// TODO(protocol): replace with generated types
export type ToolKind = 'overview' | 'metrics' | 'flux' | 'logs' | 'events' | 'portforwards'

// TODO(protocol): replace with generated types (SectionKey enum from src/protocol/v1/route.rs)
export type SectionKey = string

// TODO(protocol): replace with generated types (ScopeKind from src/protocol/v1/route.rs)
export type ScopeKind = 'Namespaced' | 'Cluster'

// TODO(protocol): replace with generated types (Route from src/protocol/v1/route.rs)
export interface Route {
  context?: string | null
  section: SectionKey
  scope: ScopeKind
  namespace?: string | null
}

// TODO(protocol): replace with generated types (context listing payload)
export interface ContextInfo {
  name: string
  cluster?: string
  user?: string
  namespace?: string
}

// TODO(protocol): replace with generated types (ThemePrefs, T015)
export interface ThemePrefs {
  mode: 'dark' | 'light'
  accent: 'cyan' | 'magenta'
  effects: 'on' | 'off'
  glitch: boolean
}

// TODO(protocol): replace with generated types (SlotConfig, T015)
export interface SlotConfig {
  kind: 'nav' | 'list' | 'inspector' | 'tool'
  sizePct: number
  pinnedRoute?: Route
  tool?: ToolKind
}

// TODO(protocol): replace with generated types (DockConfig, T015)
export interface DockConfig {
  open: boolean
  heightPct: number
  tabs: ToolKind[]
}

// TODO(protocol): replace with generated types (LayoutProfile, T015)
export interface LayoutProfile {
  slots: SlotConfig[]
  dock?: DockConfig
}

// TODO(protocol): replace with generated types (UiConfig, T015)
export interface UiConfig {
  schema: 1
  theme: ThemePrefs
  density: { override?: 'comfortable' | 'compact' }
  layouts: Partial<Record<WidthClass, LayoutProfile>>
  lastSelection?: { context: string; namespace?: string; section: SectionKey }
}
