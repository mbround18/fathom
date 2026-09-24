import type { LintInput } from '@/linter/types'

export interface Fixture {
  lists?: Record<string, { status?: 'ready' | 'loading' | 'error'; rows: unknown[] }>
  /** key `${kind}|${ns ?? '*'}|${name}` -> raw object (serialized into `raw_json`) */
  details?: Record<string, { raw: unknown; extra?: Record<string, unknown> }>
}

export function input(f: Fixture): LintInput {
  return {
    rows: (section) => {
      const l = f.lists?.[section]
      return l ? { status: l.status ?? 'ready', rows: l.rows } : undefined
    },
    detail: (kind, ns, name) => {
      const d = f.details?.[`${kind}|${ns ?? '*'}|${name}`]
      return d ? { data: { ...d.extra, raw_json: JSON.stringify(d.raw) } } : undefined
    },
  }
}

export const bare = { name: 'app', image: 'nginx' }
export const good = {
  name: 'app',
  image: 'nginx',
  resources: { requests: { cpu: '1', memory: '1' }, limits: { cpu: '1', memory: '1' } },
  livenessProbe: {},
  readinessProbe: {},
}

export function podTemplate(containers: unknown[], extra: Record<string, unknown> = {}, labels = { app: 'web' }) {
  return { metadata: { name: 'w' }, spec: { template: { metadata: { labels }, spec: { containers, ...extra } } } }
}

export function must<T>(v: T | undefined): T {
  if (v === undefined) throw new Error('expected a value')
  return v
}
