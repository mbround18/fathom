/**
 * Minimal JSON → YAML for the inspector's YAML tab. Protocol v1 has no YAML command; the read
 * replies carry `raw_json`, which this renders the way `kubectl get -o yaml` would.
 */

const PLAIN = /^[A-Za-z0-9_./:@-][A-Za-z0-9_ ./:@-]*$/
const RESERVED = /^(true|false|null|yes|no|on|off|~|-?\d+(\.\d+)?([eE][-+]?\d+)?)$/i

function scalar(v: unknown): string {
  if (v === null || v === undefined) return 'null'
  if (typeof v === 'number' || typeof v === 'boolean') return String(v)
  const s = typeof v === 'string' ? v : JSON.stringify(v)
  if (s === '') return "''"
  if (PLAIN.test(s) && !RESERVED.test(s) && !s.endsWith(' ') && !s.endsWith(':')) return s
  return JSON.stringify(s)
}

function isObject(v: unknown): v is Record<string, unknown> {
  return typeof v === 'object' && v !== null && !Array.isArray(v)
}

function emit(v: unknown, indent: string, out: string[]): void {
  if (Array.isArray(v)) {
    for (const item of v) {
      if (isObject(item) && Object.keys(item).length > 0) {
        const lines: string[] = []
        emit(item, `${indent}  `, lines)
        const [first, ...rest] = lines
        out.push(`${indent}- ${(first ?? '').trimStart()}`, ...rest)
      } else if (Array.isArray(item) && item.length > 0) {
        out.push(`${indent}-`)
        emit(item, `${indent}  `, out)
      } else {
        out.push(`${indent}- ${inline(item)}`)
      }
    }
    return
  }
  if (isObject(v)) {
    for (const [k, val] of Object.entries(v)) {
      const key = scalar(k)
      if (
        (isObject(val) && Object.keys(val).length > 0) ||
        (Array.isArray(val) && val.length > 0)
      ) {
        out.push(`${indent}${key}:`)
        emit(val, Array.isArray(val) ? indent : `${indent}  `, out)
      } else if (typeof val === 'string' && val.includes('\n')) {
        out.push(`${indent}${key}: |`)
        for (const line of val.replace(/\n$/, '').split('\n')) out.push(`${indent}  ${line}`)
      } else {
        out.push(`${indent}${key}: ${inline(val)}`)
      }
    }
    return
  }
  out.push(`${indent}${inline(v)}`)
}

function inline(v: unknown): string {
  if (Array.isArray(v)) return '[]'
  if (isObject(v)) return '{}'
  return scalar(v)
}

export function toYaml(value: unknown): string {
  const out: string[] = []
  emit(value, '', out)
  return out.join('\n')
}

/** `raw_json` → YAML; falls back to the raw text when it is not valid JSON. */
export function jsonToYaml(raw: string): string {
  try {
    return toYaml(JSON.parse(raw) as unknown)
  } catch {
    return raw
  }
}
