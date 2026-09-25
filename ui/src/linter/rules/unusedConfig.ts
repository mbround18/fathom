/**
 * ConfigMaps/Secrets not referenced by any cached Pod or workload template (env, envFrom, volumes,
 * projected volumes, imagePullSecrets). Only names/types are read; Secret contents never are.
 */
import type { Finding, Rule } from '../types'
import {
  arr,
  containersOf,
  isObj,
  makeFinding,
  missingReason,
  outcome,
  parseRaw,
  podSpecOf,
  readyRows,
  ref,
  SECTIONS,
  skipped,
  WORKLOADS,
  type Obj,
} from '../util'

const EXEMPT_SECRET_TYPES = new Set([
  'kubernetes.io/service-account-token',
  'helm.sh/release.v1',
  'bootstrap.kubernetes.io/token',
  'kubernetes.io/tls',
])
const EXEMPT_CONFIGMAPS = new Set(['kube-root-ca.crt'])

export interface ConfigRefs {
  configMaps: Set<string>
  secrets: Set<string>
}

function add(set: Set<string>, ns: string | null, name: unknown): void {
  if (typeof name === 'string') set.add(`${ns ?? '*'}/${name}`)
}

function collectVolume(v: Obj, ns: string | null, out: ConfigRefs): void {
  if (isObj(v.configMap)) add(out.configMaps, ns, v.configMap.name)
  if (isObj(v.secret)) add(out.secrets, ns, v.secret.secretName)
  if (isObj(v.projected)) {
    for (const src of arr(v.projected.sources)) {
      if (!isObj(src)) continue
      if (isObj(src.configMap)) add(out.configMaps, ns, src.configMap.name)
      if (isObj(src.secret)) add(out.secrets, ns, src.secret.name)
    }
  }
}

/** Collect every ConfigMap/Secret a pod spec references. */
export function collectRefs(spec: Obj | undefined, ns: string | null, out: ConfigRefs): void {
  if (!spec) return
  for (const v of arr(spec.volumes)) if (isObj(v)) collectVolume(v, ns, out)
  for (const s of arr(spec.imagePullSecrets)) if (isObj(s)) add(out.secrets, ns, s.name)
  const containers = [
    ...containersOf(spec),
    ...arr(spec.initContainers).filter(isObj),
    ...arr(spec.ephemeralContainers).filter(isObj),
  ]
  for (const c of containers) {
    for (const e of arr(c.envFrom)) {
      if (!isObj(e)) continue
      if (isObj(e.configMapRef)) add(out.configMaps, ns, e.configMapRef.name)
      if (isObj(e.secretRef)) add(out.secrets, ns, e.secretRef.name)
    }
    for (const e of arr(c.env)) {
      if (!isObj(e) || !isObj(e.valueFrom)) continue
      if (isObj(e.valueFrom.configMapKeyRef))
        add(out.configMaps, ns, e.valueFrom.configMapKeyRef.name)
      if (isObj(e.valueFrom.secretKeyRef)) add(out.secrets, ns, e.valueFrom.secretKeyRef.name)
    }
  }
}

export const unusedConfig: Rule = {
  id: 'unused-config',
  category: 'unused-object',
  severity: 'low',
  title: 'Unused ConfigMaps and Secrets',
  run(input) {
    const cms = readyRows(input, SECTIONS.configmaps)
    const secrets = readyRows(input, SECTIONS.secrets)
    if (!cms && !secrets)
      return skipped(this, missingReason([SECTIONS.configmaps, SECTIONS.secrets]))
    const refs: ConfigRefs = { configMaps: new Set(), secrets: new Set() }
    const missing: string[] = []
    let pending = 0
    const sources: { kind: string; section: string }[] = [
      { kind: 'Pod', section: SECTIONS.pods },
      ...WORKLOADS,
    ]
    for (const { kind, section } of sources) {
      const rows = readyRows(input, section)
      if (!rows) {
        // Without this source we cannot prove anything is unused.
        return skipped(this, missingReason([section]))
      }
      for (const row of rows) {
        const d = input.detail(kind, row.namespace, row.name)
        const raw = d ? parseRaw(d.data) : undefined
        if (!raw) pending++
        else collectRefs(podSpecOf(kind, raw), row.namespace, refs)
      }
    }
    if (!cms) missing.push(SECTIONS.configmaps)
    if (!secrets) missing.push(SECTIONS.secrets)
    const findings: Finding[] = []
    let checked = 0
    // Hold verdicts until every referencing object has been read, so nothing is falsely reported.
    const settled = pending === 0
    for (const row of cms ?? []) {
      if (EXEMPT_CONFIGMAPS.has(row.name)) continue
      checked++
      if (settled && !refs.configMaps.has(`${row.namespace ?? '*'}/${row.name}`)) {
        findings.push(
          makeFinding(
            this,
            ref(SECTIONS.configmaps, 'ConfigMap', row.namespace, row.name),
            'ConfigMap is not referenced by any Pod or workload',
          ),
        )
      }
    }
    const rawSecrets = input.rows(SECTIONS.secrets)?.rows ?? []
    const secretType = new Map<string, string>()
    for (const r of rawSecrets) {
      if (isObj(r) && typeof r.name === 'string') {
        secretType.set(
          `${String(r.namespace)}/${r.name}`,
          typeof r.secret_type === 'string' ? r.secret_type : '',
        )
      }
    }
    for (const row of secrets ?? []) {
      if (EXEMPT_SECRET_TYPES.has(secretType.get(`${String(row.namespace)}/${row.name}`) ?? ''))
        continue
      checked++
      if (settled && !refs.secrets.has(`${row.namespace ?? '*'}/${row.name}`)) {
        findings.push(
          makeFinding(
            this,
            ref(SECTIONS.secrets, 'Secret', row.namespace, row.name),
            'Secret is not referenced by any Pod or workload',
          ),
        )
      }
    }
    return outcome(
      this,
      findings,
      checked,
      pending,
      missing.length ? missingReason(missing) : undefined,
    )
  },
}
