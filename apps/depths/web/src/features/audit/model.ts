/** Pure helpers for the audit log view (US5, T070, T074). */
import type { AuditAction, AuditEntry, AuditFilter, AuditOutcome, Grant } from '@/protocol/generated'

import { argvLine } from '@/features/stacks/argv'

export const ACTION_LABEL: Record<AuditAction, string> = {
  up: 'Up',
  stop: 'Stop',
  restart: 'Restart',
  down: 'Down',
  pull: 'Pull',
  update: 'Update',
  save: 'Save',
  create: 'Create',
  delete: 'Delete',
  service_up: 'Start service',
  service_stop: 'Stop service',
  service_restart: 'Restart service',
  sign_in: 'Sign in',
  sign_out: 'Sign out',
  denied: 'Denied',
  shell_open: 'Open shell',
  reveal_env: 'Reveal .env',
  user_added: 'User added',
  user_changed: 'User changed',
  user_removed: 'User removed',
}

export const ACTIONS = Object.keys(ACTION_LABEL) as AuditAction[]

export function outcomeText(o: AuditOutcome): string {
  switch (o.kind) {
    case 'succeeded':
      return 'Succeeded'
    case 'failed':
      return o.exit_code === null ? 'Failed' : `Failed (exit ${String(o.exit_code)})`
    case 'denied':
      return `Denied: ${o.reason}`
  }
}

function grantText(g: Grant | null): string {
  if (g === null) return 'none'
  return g.role === 'user' ? `user on ${g.stacks.length ? g.stacks.join(', ') : 'no stacks'}` : g.role
}

/** The "what" of an entry, as plain lines: the argv run, the diff summary, the users change. */
export function detailLines(e: AuditEntry): string[] {
  const out = e.argv.map(argvLine)
  if (e.diff_summary) out.push(e.diff_summary)
  if (e.target) out.push(`${e.target.id}: ${grantText(e.target.before)} → ${grantText(e.target.after)}`)
  if (e.outcome.kind === 'failed' && e.outcome.stderr_tail) out.push(e.outcome.stderr_tail)
  return out
}

export function actorId(e: AuditEntry): string {
  return `${e.actor.provider}:${e.actor.subject}`
}

/** `GET /api/v1/audit/export` with the same filter, as a same-origin path. */
export function exportHref(f: AuditFilter): string {
  const q = new URLSearchParams()
  for (const k of ['stack', 'actor', 'action', 'from', 'to'] as const) {
    const v = f[k]
    if (v !== undefined && v !== '') q.set(k, v)
  }
  const s = q.toString()
  return s ? `/api/v1/audit/export?${s}` : '/api/v1/audit/export'
}

/** A `datetime-local` value (the viewer's zone) as RFC 3339 UTC; `undefined` when empty or bad. */
export function localToIso(v: string): string | undefined {
  if (!v) return undefined
  const d = new Date(v)
  return Number.isNaN(d.getTime()) ? undefined : d.toISOString()
}

/** RFC 3339 back to a `datetime-local` value in the viewer's zone. */
export function isoToLocal(iso: string | undefined): string {
  if (!iso) return ''
  const d = new Date(iso)
  if (Number.isNaN(d.getTime())) return ''
  const pad = (n: number): string => String(n).padStart(2, '0')
  return `${String(d.getFullYear())}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}T${pad(d.getHours())}:${pad(d.getMinutes())}`
}

/** Drops empty fields, so `{stack: ''}` asks for any stack. */
export function cleanFilter(f: AuditFilter): AuditFilter {
  const out: AuditFilter = {}
  if (f.stack) out.stack = f.stack
  if (f.actor) out.actor = f.actor
  if (f.action) out.action = f.action
  if (f.from) out.from = f.from
  if (f.to) out.to = f.to
  return out
}
