/**
 * Typed host queries on the fake backend (spec 067 US5, T043; contracts/protocol.md "Queries"),
 * answering as apps/keel/server/src/host_queries.rs does:
 *
 * - `ListHostQueries` → `HostQueries` for the host, newest first, with `query_v1` from
 *   `capability` (`true` speaks `query-v1`, `false` too old, `null` not connected).
 * - `AskHost` checks the parameter like the server (`OpError bad_params`). Not connected or too
 *   old settles at once (`disconnected`, `no_capability`); otherwise it answers the waiting query,
 *   and the runner's reply arrives as a pushed `HostQueryAnswered` when the test calls `settle`.
 *
 * Scripting: `capability[host]`, `seed(host, query)` and `settle(outcome, answer | refusal)` for
 * the oldest waiting query. Nothing here reads the clock: `asked_at` counts up from `NOW`.
 */
import type { Cmd, Event, HostQuery, QueryKind, QueryOutcome } from '../../src/protocol/wire'
import type { FakeBackend } from './fake-backend'

const BASE = Date.parse('2026-10-27T12:00:00Z')

function paramOf(kind: QueryKind): string | null {
  if (kind === 'unit_state') return 'unit'
  if (kind === 'path_free') return 'path'
  return null
}

function goodParams(kind: QueryKind, params: Record<string, string>): boolean {
  const name = paramOf(kind)
  const keys = Object.keys(params)
  if (name === null) return keys.length === 0
  const v = params[name]
  if (keys.length !== 1 || v === undefined) return false
  if (name === 'unit') return /^[A-Za-z0-9@._:-]{1,256}$/.test(v)
  return v === '/' || (v.startsWith('/') && v.slice(1).split('/').every((c) => c !== '' && c !== '.' && c !== '..'))
}

export class FakeQueries {
  /** Per host: `true` runner speaks `query-v1`, `false` too old, `null`/absent not connected. */
  capability: Record<string, boolean | null> = {}
  items: Record<string, HostQuery[]> = {}
  private seq = 0
  private readonly backend: FakeBackend
  private readonly asker: string

  constructor(backend: FakeBackend, asker = 'Ada Admin') {
    this.backend = backend
    this.asker = asker
    backend.on('ListHostQueries', (cmd) => this.respond(cmd))
    backend.on('AskHost', (cmd) => this.respond(cmd))
  }

  /** A query already on the list. */
  seed(host: string, over: Partial<HostQuery> = {}): HostQuery {
    const q = { ...this.make(host, 'facts', {}), outcome: 'answered' as const, answer: { kernel: '6.8' }, ...over }
    q.answered_at ??= q.asked_at
    this.store(q)
    return q
  }

  /** Settles the oldest waiting query and pushes `HostQueryAnswered`. */
  settle(outcome: QueryOutcome, detail: { answer?: unknown; refusal?: string } = {}): HostQuery {
    const waiting = Object.values(this.items)
      .flat()
      .filter((q) => q.outcome === null)
      .sort((a, b) => a.asked_at.localeCompare(b.asked_at))[0]
    if (!waiting) throw new Error('no waiting query')
    const done: HostQuery = {
      ...waiting,
      outcome,
      answer: detail.answer ?? null,
      refusal: detail.refusal ?? null,
      answered_at: waiting.asked_at,
    }
    this.store(done)
    this.backend.push({ type: 'HostQueryAnswered', data: done })
    return done
  }

  private make(host: string, kind: QueryKind, params: Record<string, string>): HostQuery {
    this.seq += 1
    return {
      id: `query-${String(this.seq).padStart(4, '0')}`,
      host,
      kind,
      params,
      asked_by: this.asker,
      asked_at: new Date(BASE + this.seq * 60_000).toISOString().replace('.000Z', 'Z'),
      outcome: null,
      answer: null,
      refusal: null,
      answered_at: null,
    }
  }

  private store(q: HostQuery): void {
    const rest = (this.items[q.host] ?? []).filter((i) => i.id !== q.id)
    this.items[q.host] = [q, ...rest].sort((a, b) => b.asked_at.localeCompare(a.asked_at))
  }

  private respond(cmd: Cmd): Event[] | undefined {
    switch (cmd.type) {
      case 'ListHostQueries': {
        const host = cmd.data.host
        return [{ type: 'HostQueries', data: { host, items: this.items[host] ?? [], query_v1: this.capability[host] ?? null } }]
      }
      case 'AskHost': {
        const { host, kind, params } = cmd.data
        if (!goodParams(kind, params)) return [{ type: 'OpError', data: { code: 'bad_params', message: 'bad parameter' } }]
        const q = this.make(host, kind, params)
        const cap = this.capability[host] ?? null
        if (cap !== true) {
          q.outcome = cap === null ? 'disconnected' : 'no_capability'
          q.answered_at = q.asked_at
        }
        this.store(q)
        return [{ type: 'HostQuery', data: q }]
      }
      default:
        return undefined
    }
  }
}
