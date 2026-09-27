/**
 * Pure logic for the Role/ClusterRole create + edit form (042-role-create): rule rows, verb
 * presets, manifest generation, edit-mode patching, validation, warnings and the API server's
 * privilege-escalation error. Kept out of `RoleForm.tsx` so it is unit-testable without React
 * (`ui/tests/unit/roleYaml.test.ts`), mirroring `roleBindingYaml.ts`.
 */
import type { ApiResourceCatalog } from '@/protocol/types'
import { jsonToYaml } from '@/resources/yaml'

export type RoleKind = 'Role' | 'ClusterRole'

/** One `rules[]` entry as the editor holds it. Empty arrays are omitted from the manifest. */
export interface RuleInput {
  apiGroups: string[]
  resources: string[]
  verbs: string[]
  resourceNames: string[]
  /** ClusterRole only (a Role cannot grant non-resource URLs). */
  nonResourceURLs: string[]
}

/** The verbs offered as checkboxes; anything else (`*`, `bind`, `escalate`, ...) is free text. */
export const STANDARD_VERBS = [
  'get',
  'list',
  'watch',
  'create',
  'update',
  'patch',
  'delete',
  'deletecollection',
] as const

export type PresetId = 'read-only' | 'edit' | 'custom'

export const PRESET_VERBS: Record<Exclude<PresetId, 'custom'>, readonly string[]> = {
  'read-only': ['get', 'list', 'watch'],
  edit: STANDARD_VERBS,
}

export const PRESET_LABELS: Record<PresetId, string> = {
  'read-only': 'Read-only (get, list, watch)',
  edit: 'Edit (read + create, update, patch, delete)',
  custom: 'Custom',
}

const API_VERSION = 'rbac.authorization.k8s.io/v1'

export function emptyRule(preset: PresetId = 'read-only'): RuleInput {
  return {
    apiGroups: [''],
    resources: [],
    verbs: preset === 'custom' ? [] : [...PRESET_VERBS[preset]],
    resourceNames: [],
    nonResourceURLs: [],
  }
}

function sameSet(a: readonly string[], b: readonly string[]): boolean {
  if (a.length !== b.length) return false
  const set = new Set(a)
  return b.every((v) => set.has(v))
}

/** The preset every rule's verbs match, or `custom`. No rules reads as `custom`. */
export function presetOf(rules: readonly RuleInput[]): PresetId {
  if (rules.length === 0) return 'custom'
  for (const id of ['read-only', 'edit'] as const) {
    if (rules.every((r) => sameSet(r.verbs, PRESET_VERBS[id]))) return id
  }
  return 'custom'
}

/** Set every rule's verbs to the preset's; `custom` leaves the rules unchanged. */
export function applyPreset(rules: readonly RuleInput[], preset: PresetId): RuleInput[] {
  if (preset === 'custom') return [...rules]
  return rules.map((r) => ({ ...r, verbs: [...PRESET_VERBS[preset]] }))
}

/** Toggle one verb on a rule, keeping the verbs in a stable order (checkbox order, then rest). */
export function toggleVerb(rule: RuleInput, verb: string, on: boolean): RuleInput {
  const set = new Set(rule.verbs)
  if (on) set.add(verb)
  else set.delete(verb)
  return { ...rule, verbs: orderVerbs([...set]) }
}

export function orderVerbs(verbs: readonly string[]): string[] {
  const standard = STANDARD_VERBS.filter((v) => verbs.includes(v))
  const rest = verbs.filter((v) => !(STANDARD_VERBS as readonly string[]).includes(v))
  return [...standard, ...rest]
}

/**
 * Parse a free-text token list (comma or whitespace separated). `core` and `""` both mean the
 * core API group (`''`) when `coreGroup` is set.
 */
export function parseTokens(text: string, coreGroup = false): string[] {
  return text
    .split(/[\s,]+/)
    .map((t) => t.trim())
    .filter((t) => t.length > 0)
    .map((t) => (coreGroup && (t === '""' || t === "''" || t === 'core') ? '' : t))
}

/** Display form of an API group token: the core group is shown as `core`. */
export function groupLabel(group: string): string {
  return group === '' ? 'core' : group
}

export function ruleToManifest(rule: RuleInput, kind: RoleKind): Record<string, unknown> {
  const out: Record<string, unknown> = {}
  const isResourceRule = rule.resources.length > 0 || rule.nonResourceURLs.length === 0
  if (isResourceRule) {
    out.apiGroups = [...rule.apiGroups]
    out.resources = [...rule.resources]
  }
  if (rule.resourceNames.length > 0) out.resourceNames = [...rule.resourceNames]
  if (kind === 'ClusterRole' && rule.nonResourceURLs.length > 0) {
    out.nonResourceURLs = [...rule.nonResourceURLs]
  }
  out.verbs = [...rule.verbs]
  return out
}

export interface RoleManifestInput {
  kind: RoleKind
  name: string
  /** Role only; ignored for a ClusterRole. */
  namespace: string | null
  rules: readonly RuleInput[]
  labels?: Readonly<Record<string, string>>
}

export function buildRoleManifest(input: RoleManifestInput): Record<string, unknown> {
  const metadata: Record<string, unknown> = { name: input.name }
  if (input.kind === 'Role' && input.namespace) metadata.namespace = input.namespace
  if (input.labels && Object.keys(input.labels).length > 0) metadata.labels = { ...input.labels }
  return {
    apiVersion: API_VERSION,
    kind: input.kind,
    metadata,
    rules: input.rules.map((r) => ruleToManifest(r, input.kind)),
  }
}

export function roleManifestYaml(input: RoleManifestInput): string {
  return jsonToYaml(JSON.stringify(buildRoleManifest(input)))
}

function parseObject(rawJson: string): Record<string, unknown> {
  try {
    const value: unknown = JSON.parse(rawJson)
    return typeof value === 'object' && value !== null && !Array.isArray(value)
      ? (value as Record<string, unknown>)
      : {}
  } catch {
    return {}
  }
}

function strings(v: unknown): string[] {
  return Array.isArray(v) ? v.filter((x): x is string => typeof x === 'string') : []
}

/** Edit mode: the live object's rules as editor rows (resourceNames/nonResourceURLs kept). */
export function rulesFromRawJson(rawJson: string): RuleInput[] {
  const rules = parseObject(rawJson).rules
  if (!Array.isArray(rules)) return []
  return rules.map((r) => {
    const rule = typeof r === 'object' && r !== null ? (r as Record<string, unknown>) : {}
    return {
      apiGroups: strings(rule.apiGroups),
      resources: strings(rule.resources),
      verbs: strings(rule.verbs),
      resourceNames: strings(rule.resourceNames),
      nonResourceURLs: strings(rule.nonResourceURLs),
    }
  })
}

/**
 * Edit mode: replace only `rules` in the live object, carrying every other field (metadata,
 * `resourceVersion`, `aggregationRule`, ...) through untouched so the diff shows only the edit.
 */
export function patchRoleRules(
  rawJson: string,
  kind: RoleKind,
  rules: readonly RuleInput[],
): string {
  const parsed = parseObject(rawJson)
  return jsonToYaml(JSON.stringify({ ...parsed, rules: rules.map((r) => ruleToManifest(r, kind)) }))
}

/**
 * Role names are path segments: anything but `/` and `%`, not `.`/`..`, at most 253 chars.
 * (`system:` names are valid; the form warns about them separately.)
 */
export function roleNameProblem(name: string): string | null {
  const n = name.trim()
  if (n.length === 0) return 'Name is required.'
  if (n !== name) return 'Name cannot start or end with whitespace.'
  if (n === '.' || n === '..') return `'${n}' is not a valid name.`
  if (/[/%]/.test(n)) return "Name cannot contain '/' or '%'."
  if (n.length > 253) return 'Name is longer than 253 characters.'
  return null
}

/** Per-rule problems, in the order the editor shows them. Empty when the rule is valid. */
export function ruleProblems(rule: RuleInput, kind: RoleKind): string[] {
  const problems: string[] = []
  if (rule.verbs.length === 0) problems.push('Pick at least one verb.')
  const nonResource = kind === 'ClusterRole' && rule.nonResourceURLs.length > 0
  if (rule.resources.length > 0 && rule.apiGroups.length === 0) {
    problems.push('Resource rules need at least one API group (core is "core").')
  }
  if (rule.resources.length === 0 && !nonResource) {
    problems.push(
      kind === 'ClusterRole'
        ? 'Add at least one resource (or a non-resource URL).'
        : 'Add at least one resource.',
    )
  }
  if (nonResource && rule.resources.length > 0) {
    problems.push('A rule grants either resources or non-resource URLs, not both.')
  }
  if (nonResource && rule.resourceNames.length > 0) {
    problems.push('Resource names only apply to resource rules.')
  }
  return problems
}

export function rulesValid(rules: readonly RuleInput[], kind: RoleKind): boolean {
  return rules.length > 0 && rules.every((r) => ruleProblems(r, kind).length === 0)
}

export interface RoleWarnings {
  /** `system:` roles are Kubernetes' own; many are reconciled by the API server on start. */
  system: boolean
  /** The ClusterRole has an `aggregationRule`: its rules are controller-managed. */
  aggregated: boolean
}

export function roleWarnings(name: string, rawJson?: string): RoleWarnings {
  const aggregationRule = rawJson ? parseObject(rawJson).aggregationRule : undefined
  return {
    system: name.startsWith('system:'),
    aggregated: aggregationRule !== undefined && aggregationRule !== null,
  }
}

/** The API server's RBAC escalation-prevention error ("attempting to grant ... not held"). */
export function isEscalationError(message: string | undefined): boolean {
  if (!message) return false
  return /attempting to grant RBAC permissions not currently held/i.test(message)
}

/** The `{APIGroups:[...], Resources:[...], Verbs:[...]}` entries the API server listed. */
export function escalationMissing(message: string): string[] {
  return [...message.matchAll(/\{APIGroups:[^}]*\}|\{NonResourceURLs:[^}]*\}/g)].map((m) => m[0])
}

// ------------------------------------------------------------------ discovery-fed pickers

/** Every discovered API group (core first), for the API group picker. */
export function groupOptions(catalog: ApiResourceCatalog | undefined): string[] {
  return (catalog?.groups ?? []).map((g) => g.group)
}

/**
 * Resource names for the resource picker: the selected groups' resources (every group's when
 * none or `*` is selected). A Role only offers namespaced resources: cluster-scoped ones in a
 * Role grant nothing.
 */
export function resourceOptions(
  catalog: ApiResourceCatalog | undefined,
  groups: readonly string[],
  kind: RoleKind,
): string[] {
  const all = groups.length === 0 || groups.includes('*')
  const names = new Set<string>()
  for (const g of catalog?.groups ?? []) {
    if (!all && !groups.includes(g.group)) continue
    for (const r of g.resources) {
      if (kind === 'Role' && !r.namespaced) continue
      names.add(r.name)
    }
  }
  return [...names].sort()
}
