/**
 * Pure YAML-generation logic for RoleBinding/ClusterRoleBinding create + edit
 * (011-rbac-oidc-administration US2 T022, US3 T031-T032). Kept out of `RoleBindingForm.tsx` so
 * it is unit-testable without mounting React (`ui/src/resources/__tests__/rolebindingForm.test.ts`).
 */
import { jsonToYaml } from '@/resources/yaml'

export interface RoleBindingSubjectInput {
  kind: string
  name: string
  namespace: string | null
}

export interface RoleBindingManifestInput {
  kind: 'RoleBinding' | 'ClusterRoleBinding'
  name: string
  /** RoleBinding only; ignored for a ClusterRoleBinding (which has no namespace). */
  namespace: string | null
  roleRefKind: 'Role' | 'ClusterRole'
  roleRefName: string
  subjects: readonly RoleBindingSubjectInput[]
  /** Optional `metadata.labels` (create mode). Omitted from the manifest when empty. */
  labels?: Readonly<Record<string, string>>
}

/** A label key: optional DNS-subdomain prefix + `/`, then a 1-63 char name (k8s rules). */
const LABEL_KEY =
  /^([a-z0-9]([-a-z0-9]*[a-z0-9])?(\.[a-z0-9]([-a-z0-9]*[a-z0-9])?)*\/)?[A-Za-z0-9]([-A-Za-z0-9_.]{0,61}[A-Za-z0-9])?$/
/** A label value: empty, or 1-63 chars, alphanumeric at both ends. */
const LABEL_VALUE = /^([A-Za-z0-9]([-A-Za-z0-9_.]{0,61}[A-Za-z0-9])?)?$/

/**
 * Parse the form's `key=value, key2=value2` labels field. Returns the labels, or an error
 * naming the first invalid entry. Blank input is `{}`.
 */
export function parseLabels(
  input: string,
): { ok: true; labels: Record<string, string> } | { ok: false; error: string } {
  const labels: Record<string, string> = {}
  for (const raw of input.split(',')) {
    const entry = raw.trim()
    if (entry === '') continue
    const eq = entry.indexOf('=')
    if (eq <= 0) return { ok: false, error: `'${entry}' is not key=value` }
    const key = entry.slice(0, eq).trim()
    const value = entry.slice(eq + 1).trim()
    if (key.length > 316 || !LABEL_KEY.test(key)) {
      return { ok: false, error: `'${key}' is not a valid label key` }
    }
    if (!LABEL_VALUE.test(value)) {
      return { ok: false, error: `'${value}' is not a valid label value` }
    }
    labels[key] = value
  }
  return { ok: true, labels }
}

const API_VERSION = 'rbac.authorization.k8s.io/v1'
const API_GROUP = 'rbac.authorization.k8s.io'

function subjectToManifest(s: RoleBindingSubjectInput): Record<string, unknown> {
  const out: Record<string, unknown> = { kind: s.kind, name: s.name }
  // ServiceAccount subjects require `namespace` on the wire; User/Group subjects have none
  // (SubjectSummary carries `namespace: null` for those, which this mirrors by omission).
  if (s.kind === 'ServiceAccount') out.namespace = s.namespace ?? ''
  return out
}

/**
 * Builds a plain JS object matching the shape a real RoleBinding/ClusterRoleBinding manifest
 * would have (T022): `apiVersion`/`kind`/`metadata.name`(+`namespace` for a RoleBinding)/
 * `roleRef.{apiGroup,kind,name}`/`subjects[].{kind,name,namespace?}` — the same fields
 * `RoleBindingDetails`/`ClusterRoleBindingDetails` represent.
 */
export function buildRoleBindingManifest(input: RoleBindingManifestInput): Record<string, unknown> {
  const metadata: Record<string, unknown> = { name: input.name }
  if (input.kind === 'RoleBinding' && input.namespace) metadata.namespace = input.namespace
  if (input.labels && Object.keys(input.labels).length > 0) metadata.labels = { ...input.labels }

  return {
    apiVersion: API_VERSION,
    kind: input.kind,
    metadata,
    roleRef: {
      apiGroup: API_GROUP,
      kind: input.roleRefKind,
      name: input.roleRefName,
    },
    subjects: input.subjects.map(subjectToManifest),
  }
}

/** `buildRoleBindingManifest` rendered as YAML, for a fresh create-mode edit session buffer. */
export function roleBindingManifestYaml(input: RoleBindingManifestInput): string {
  return jsonToYaml(JSON.stringify(buildRoleBindingManifest(input)))
}

/**
 * Edit mode (US3 T031-T032): patches only `roleRef`/`subjects` into the existing live object's
 * parsed `raw_json`, leaving every other field (metadata, labels, annotations,
 * `resourceVersion`, ...) untouched — so the diff view shows only the operator's actual change,
 * not a full-object rewrite. Falls back to an empty object if `rawJson` fails to parse (should
 * not happen: it is always seeded from a `RoleBindingDetails`/`ClusterRoleBindingDetails` reply).
 */
export function patchRoleBindingSubjects(
  rawJson: string,
  roleRefKind: 'Role' | 'ClusterRole',
  roleRefName: string,
  subjects: readonly RoleBindingSubjectInput[],
): string {
  let parsed: Record<string, unknown>
  try {
    const value: unknown = JSON.parse(rawJson)
    parsed = typeof value === 'object' && value !== null ? (value as Record<string, unknown>) : {}
  } catch {
    parsed = {}
  }
  return jsonToYaml(
    JSON.stringify({
      ...parsed,
      roleRef: { apiGroup: API_GROUP, kind: roleRefKind, name: roleRefName },
      subjects: subjects.map(subjectToManifest),
    }),
  )
}
