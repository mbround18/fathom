/**
 * Spec 068 wording and checks for runs (T044, T051, T060, T066), kept apart from the components so
 * Vitest covers them: required-check states, the narrowing note, a covered or skipped host, and the
 * Start dialog's params.
 */
import type { CheckState, ParamView, RunCheckView } from '@/protocol/wire'

/** A required check's state, in words. */
export function checkLabel(state: CheckState): string {
  switch (state) {
    case 'success':
      return 'passed'
    case 'failure':
      return 'failed'
    case 'error':
      return 'errored'
    case 'pending':
      return 'pending'
    case 'missing':
      return 'not reported yet'
  }
}

export type CheckTone = 'ok' | 'err' | 'wait'

export function checkTone(state: CheckState): CheckTone {
  if (state === 'success') return 'ok'
  if (state === 'failure' || state === 'error') return 'err'
  return 'wait'
}

/** "2 of 3 required checks passed". */
export function checksSummary(checks: readonly RunCheckView[]): string {
  const passed = checks.filter((c) => c.state === 'success').length
  const failed = checks.filter((c) => checkTone(c.state) === 'err').length
  const total = checks.length
  const noun = total === 1 ? 'required check' : 'required checks'
  if (failed > 0) return `${String(failed)} of ${String(total)} ${noun} failed`
  return `${String(passed)} of ${String(total)} ${noun} passed`
}

/** Only `http(s)` links from the forge become links; anything else stays text. */
export function forgeLink(url: string | null): string | null {
  if (url === null) return null
  try {
    const u = new URL(url)
    return u.protocol === 'https:' || u.protocol === 'http:' ? u.toString() : null
  } catch {
    return null
  }
}

/** "group_vars/keel_servers.yml": the path-triggered run's narrowing (FR-020), as one line. */
export function narrowedText(paths: readonly string[]): string {
  const noun = paths.length === 1 ? 'this vars file' : 'these vars files'
  return `Only the hosts ${noun} names: ${paths.join(', ')}`
}

/** Why a converge step was skipped, in words. */
export function skipText(reason: string | null): string {
  if (!reason) return 'skipped'
  if (reason === 'dependency_failed') return 'skipped: an earlier step failed'
  return `skipped: ${reason.replace(/_/g, ' ')}`
}

// ---- Start dialog params (T066) -----------------------------------------------------------------

const DIGEST = /^sha256:[0-9a-f]{64}$/

/** Why a `choice` param can't be listed, in words. */
export function unavailableText(code: string): string {
  switch (code) {
    case 'choice_from_missing':
      return 'Its folder is missing at this commit, so no value can be picked.'
    case 'choice_from_too_many':
      return 'Its folder has too many entries to list, so no value can be picked.'
    case 'choice_shape':
      return 'Its declaration in .keel/jobs.yaml is malformed, so no value can be picked.'
    default:
      return `No value can be picked (${code.replace(/_/g, ' ')}).`
  }
}

/** The values a form starts with: a `choice` param's default when it is allowed, else empty. */
export function initialParams(params: Readonly<Record<string, ParamView>> | undefined): Record<string, string> {
  const out: Record<string, string> = {}
  for (const [name, view] of Object.entries(params ?? {})) {
    const d = view.default
    out[name] = view.kind === 'choice' && d !== null && (view.allowed ?? []).includes(d) ? d : ''
  }
  return out
}

/** What is wrong with `value` for `view`, or `null` when it can be sent. */
export function paramProblem(view: ParamView, value: string): string | null {
  if (view.kind === 'choice') {
    if (view.unavailable) return unavailableText(view.unavailable)
    const allowed = view.allowed ?? []
    if (allowed.length === 0) return 'There is nothing to pick from at this commit.'
    if (value === '') return 'Pick a value.'
    return allowed.includes(value) ? null : 'Pick one of the listed values.'
  }
  if (value.trim() === '') return 'Enter an image digest.'
  return DIGEST.test(value.trim()) ? null : 'An image digest is sha256: and 64 lowercase hex characters.'
}

/** Params sorted by name, as the form lists them. */
export function paramNames(params: Readonly<Record<string, ParamView>> | undefined): string[] {
  return Object.keys(params ?? {}).sort((a, b) => a.localeCompare(b))
}

/** Whether every param can be sent. */
export function paramsReady(params: Readonly<Record<string, ParamView>> | undefined, values: Readonly<Record<string, string>>): boolean {
  return paramNames(params).every((n) => {
    const view = params?.[n]
    return view !== undefined && paramProblem(view, values[n] ?? '') === null
  })
}

/** The values to send: trimmed, every declared param once. */
export function paramsToSend(params: Readonly<Record<string, ParamView>> | undefined, values: Readonly<Record<string, string>>): Record<string, string> {
  const out: Record<string, string> = {}
  for (const n of paramNames(params)) out[n] = (values[n] ?? '').trim()
  return out
}

// ---- Required checks on a binding (T060) --------------------------------------------------------

/** Server limits (server/src/checks.rs): at most 16 contexts of 1 to 200 characters, timeout 60 s to 24 h. */
export const MAX_CONTEXTS = 16
export const MIN_TIMEOUT_MIN = 1
export const MAX_TIMEOUT_MIN = 24 * 60

/** "30 min", "2 h", "1 h 30 min". */
export function timeoutText(secs: number): string {
  const min = Math.round(secs / 60)
  const h = Math.floor(min / 60)
  const m = min % 60
  if (h === 0) return `${String(m)} min`
  return m === 0 ? `${String(h)} h` : `${String(h)} h ${String(m)} min`
}

/** A context as typed, or why it can't be added. */
export function contextProblem(context: string, existing: readonly string[]): string | null {
  const c = context.trim()
  if (c === '') return 'Enter a status context, e.g. ci/build.'
  if (Array.from(c).length > 200) return 'A context is at most 200 characters.'
  // eslint-disable-next-line no-control-regex
  if (/[\u0000-\u001f\u007f]/.test(c)) return 'A context is printable characters only.'
  if (existing.includes(c)) return `${c} is already required.`
  if (existing.length >= MAX_CONTEXTS) return `At most ${String(MAX_CONTEXTS)} contexts.`
  return null
}
