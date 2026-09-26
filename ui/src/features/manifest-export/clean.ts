/**
 * 040-manifest-export: clean-for-re-apply rules and YAML rendering. Normative rules are in
 * `specs/040-manifest-export/contracts/clean-rules.md` (C1-C8); each has a Vitest case.
 *
 * YAML is emitted with the YAML 1.1 schema, which is what `kubectl` (go-yaml) reads: strings
 * such as `yes`, `on`, `0755` or `2026-01-01` get quoted, so they stay strings (FR-013).
 */
import { stringify } from 'yaml'

type Json = Record<string, unknown>

const METADATA_FIELDS = [
  'managedFields',
  'resourceVersion',
  'uid',
  'creationTimestamp',
  'generation',
  'selfLink',
  'deletionTimestamp',
  'deletionGracePeriodSeconds',
  'ownerReferences',
] as const

export const SERVER_ANNOTATIONS: readonly string[] = [
  'kubectl.kubernetes.io/last-applied-configuration',
  'deployment.kubernetes.io/revision',
  'pv.kubernetes.io/bind-completed',
  'pv.kubernetes.io/bound-by-controller',
  'pv.kubernetes.io/provisioned-by',
  'volume.beta.kubernetes.io/storage-provisioner',
  'volume.kubernetes.io/storage-provisioner',
]

const JOB_GENERATED_LABELS = [
  'controller-uid',
  'batch.kubernetes.io/controller-uid',
  'job-name',
  'batch.kubernetes.io/job-name',
]

export const REDACTED = '<redacted>'

function isObject(v: unknown): v is Json {
  return typeof v === 'object' && v !== null && !Array.isArray(v)
}

function objectAt(root: Json, ...path: string[]): Json | undefined {
  let cur: unknown = root
  for (const key of path) {
    if (!isObject(cur)) return undefined
    cur = cur[key]
  }
  return isObject(cur) ? cur : undefined
}

/** C7: `apiVersion`, `kind`, `metadata` first, the rest in their original order. */
function ordered(obj: Json): Json {
  const out: Json = {}
  for (const k of ['apiVersion', 'kind', 'metadata']) if (k in obj) out[k] = obj[k]
  for (const [k, v] of Object.entries(obj)) if (!(k in out)) out[k] = v
  return out
}

export interface CleanResult {
  object: Json
  warnings: string[]
}

/** Apply C1-C8. The input is never mutated (C8). */
export function cleanManifest(input: Json): CleanResult {
  const obj = structuredClone(input)
  const warnings: string[] = []

  delete obj.status // C1

  const metadata = objectAt(obj, 'metadata')
  if (metadata) {
    // C3: warn before the refs go.
    const refs = Array.isArray(metadata.ownerReferences) ? metadata.ownerReferences : []
    for (const ref of refs) {
      if (isObject(ref) && ref.controller === true) {
        warnings.push(
          `Owned by ${String(ref.kind)} ${String(ref.name)}: re-applying creates an unmanaged copy the controller does not own.`,
        )
      }
    }
    // eslint-disable-next-line @typescript-eslint/no-dynamic-delete -- fixed field list
    for (const f of METADATA_FIELDS) delete metadata[f] // C2, C3
    const annotations = objectAt(metadata, 'annotations')
    if (annotations) {
      // eslint-disable-next-line @typescript-eslint/no-dynamic-delete -- fixed annotation list
      for (const a of SERVER_ANNOTATIONS) delete annotations[a] // C4
      if (Object.keys(annotations).length === 0) delete metadata.annotations
    }
  }

  const spec = objectAt(obj, 'spec')
  if (obj.kind === 'Service' && spec && spec.clusterIP !== 'None') {
    delete spec.clusterIP // C5
    delete spec.clusterIPs
  }
  if (obj.kind === 'Job' && spec && spec.manualSelector !== true) {
    delete spec.selector // C6
    const labels = objectAt(spec, 'template', 'metadata', 'labels')
    // eslint-disable-next-line @typescript-eslint/no-dynamic-delete -- fixed label list
    if (labels) for (const l of JOB_GENERATED_LABELS) delete labels[l]
  }

  return { object: ordered(obj), warnings }
}

export interface RenderedManifest {
  yaml: string
  warnings: string[]
}

function toYaml(obj: unknown): string {
  return stringify(obj, { lineWidth: 0, version: '1.1' })
}

/** One object's YAML, clean or raw, plus the warnings the dialog shows for it. */
export function renderManifest(raw: string | Json, opts: { clean: boolean }): RenderedManifest {
  const parsed: unknown = typeof raw === 'string' ? JSON.parse(raw) : raw
  if (!isObject(parsed)) return { yaml: toYaml(parsed), warnings: [] }
  const { object, warnings } = opts.clean
    ? cleanManifest(parsed)
    : { object: structuredClone(parsed), warnings: [] as string[] }
  if (object.kind === 'Secret' && object.apiVersion === 'v1') {
    warnings.push('Secret values are redacted: fill them in before applying.')
  }
  return { yaml: toYaml(object), warnings }
}

/** Multi-document YAML: `---` between documents, one trailing newline. */
export function renderBundle(docs: readonly string[]): string {
  if (docs.length === 0) return ''
  return `${docs.map((d) => d.replace(/\n+$/, '')).join('\n---\n')}\n`
}

function slug(s: string): string {
  return s
    .toLowerCase()
    .replace(/[^a-z0-9.-]+/g, '-')
    .replace(/^-+|-+$/g, '')
}

/** FR-005: `<kind>-<namespace>-<name>.yaml`, or `<kind>-<count>-objects.yaml` for a bundle. */
export function exportFilename(
  kind: string,
  objects: readonly { namespace?: string | null; name: string }[],
): string {
  const k = slug(kind) || 'objects'
  const [only] = objects
  if (objects.length === 1 && only) {
    const parts = [k, only.namespace ? slug(only.namespace) : undefined, slug(only.name)]
    return `${parts.filter((p): p is string => p !== undefined && p !== '').join('-')}.yaml`
  }
  return `${k}-${String(objects.length)}-objects.yaml`
}

/** FR-015: bundles are sorted by namespace (cluster-scoped first), then name. */
export function compareRefs(
  a: { namespace?: string | null; name: string },
  b: { namespace?: string | null; name: string },
): number {
  return (a.namespace ?? '').localeCompare(b.namespace ?? '') || a.name.localeCompare(b.name)
}
