/**
 * The enrolment snippet template, mirrored from `keel_core::enroll_template` (spec 060). The
 * server is the authority; this twin gives the editor inline errors and a live preview without a
 * round trip. Both are tested against `contracts/template-vectors.json`.
 *
 * Grammar: `%t` token (required), `%n` inventory name, `%p` profiles, `%w` workspace slug, `%s`
 * runner address, `%u` public URL, `%e` expiry, `%%` a literal percent. Anything else after `%`,
 * including a trailing `%`, is an unknown placeholder.
 */

export const MAX_BYTES = 4096

export const DEFAULT_TEMPLATE = 'read -rs KEEL_TOKEN; export KEEL_TOKEN  # paste: %t\nmake keel-node'

/** Shown for `%s` while no runner address is configured. */
export const NO_RUNNER_ADDRESS = '<runner address not set>'

export type Placeholder = 't' | 'n' | 'p' | 'w' | 's' | 'u' | 'e'

export const PLACEHOLDERS: readonly { key: Placeholder; meaning: string }[] = [
  { key: 't', meaning: 'Enrolment token (required)' },
  { key: 'n', meaning: 'Inventory name' },
  { key: 'p', meaning: 'Profiles, comma separated' },
  { key: 'w', meaning: 'Workspace slug' },
  { key: 's', meaning: 'Runner address (host:port)' },
  { key: 'u', meaning: 'Public URL' },
  { key: 'e', meaning: 'Token expiry (RFC 3339)' },
]

export type TemplateValues = Record<Placeholder, string>

export type TemplateError =
  | { code: 'too_long'; bytes: number }
  | { code: 'unknown_placeholder'; placeholder: string }
  | { code: 'missing_token' }

const KNOWN = new Set<string>(['t', 'n', 'p', 'w', 's', 'u', 'e'])

export function byteLength(s: string): number {
  return new TextEncoder().encode(s).length
}

/** The first problem, in the server's order: too long, then unknown placeholder, then no `%t`. */
export function validate(template: string): TemplateError | null {
  const bytes = byteLength(template)
  if (bytes > MAX_BYTES) return { code: 'too_long', bytes }
  let token = false
  // Code units suffice: `%` and the placeholder letters are ASCII, and every other unit is copied.
  for (let i = 0; i < template.length; i++) {
    if (template[i] !== '%') continue
    const next = template[i + 1]
    if (next === '%') {
      i++
      continue
    }
    if (next !== undefined && KNOWN.has(next)) {
      if (next === 't') token = true
      i++
      continue
    }
    return { code: 'unknown_placeholder', placeholder: next === undefined ? '%' : `%${next}` }
  }
  return token ? null : { code: 'missing_token' }
}

/** One pass: substituted values are never re-scanned. Unknown placeholders are copied through. */
export function render(template: string, values: TemplateValues): string {
  let out = ''
  for (let i = 0; i < template.length; i++) {
    const c = template.charAt(i)
    if (c !== '%') {
      out += c
      continue
    }
    const next = template[i + 1]
    if (next === undefined) {
      out += '%'
    } else if (next === '%') {
      out += '%'
      i++
    } else if (KNOWN.has(next)) {
      out += values[next as Placeholder]
      i++
    } else {
      out += `%${next}`
      i++
    }
  }
  return out
}

export function errorText(e: TemplateError): string {
  switch (e.code) {
    case 'too_long':
      return `The template is ${String(e.bytes)} bytes; the limit is ${String(MAX_BYTES)}.`
    case 'unknown_placeholder':
      return e.placeholder === '%'
        ? 'A trailing % needs a letter after it. Write %% for a literal percent.'
        : `${e.placeholder} is not a placeholder. Write %% for a literal percent.`
    case 'missing_token':
      return 'The template must include %t, the enrolment token.'
  }
}
