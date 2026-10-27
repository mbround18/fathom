/**
 * The fake backend's inventories (spec 055 US4, T055), beside ./fake-repos.ts.
 *
 * An in-memory model answering the inventory commands as apps/keel/server/src/inventory.rs does:
 * revisions are append-only; an edit or a restore previews, then `Confirm` adds the next revision
 * and answers `InventoryRevision`. Edits from a stale base, edits with errors and edits that change
 * nothing are refused with the server's codes.
 *
 * It reads only a small YAML subset (`key: value` lines nested by two-space indent), enough for the
 * specs: a host is a key under a group's `hosts`, and every inventory checks one var against a
 * schema: `http_port` must be an integer.
 */
import type {
  Cmd,
  Event,
  InventoryChange,
  InventoryHostInfo,
  InventoryInfo,
  InventoryProblem,
  InventoryRevisionInfo,
  InventoryRevisionSummary,
} from '../../src/protocol/wire'
import { NOW, WORKSPACE } from './fake-repos'

const EXPIRES = '2026-10-27T12:10:00Z'

type Pending = { apply: () => Event[] }

interface Checked {
  /** Dotted path to leaf value, as JSON text. */
  leaves: Map<string, string>
  hosts: InventoryHostInfo[]
  errors: InventoryProblem[]
  hash: string
}

interface Stored {
  info: InventoryInfo
  revisions: InventoryRevisionInfo[]
}

/** Reads the YAML subset the fake understands. */
export function check(source: string): Checked {
  const leaves = new Map<string, string>()
  const errors: InventoryProblem[] = []
  const hosts = new Map<string, Set<string>>()
  const stack: string[] = []
  source.split('\n').forEach((raw, i) => {
    const line = raw.replace(/\s+#.*$/, '')
    if (line.trim() === '' || line.trimStart().startsWith('#')) return
    const m = /^( *)([A-Za-z0-9_.-]+):(?: (.*))?$/.exec(line)
    if (!m) {
      errors.push({ at: `line ${String(i + 1)}`, message: 'expected `key: value`' })
      return
    }
    const depth = Math.floor((m[1] ?? '').length / 2)
    stack.length = depth
    stack.push(m[2] ?? '')
    const path = stack.join('.')
    if (depth === 2 && stack[1] === 'hosts') {
      const group = stack[0] ?? ''
      const host = stack[2] ?? ''
      hosts.set(host, (hosts.get(host) ?? new Set()).add(group))
    }
    const value = (m[3] ?? '').trim()
    if (value === '' || value === '{}') return
    const number = /^-?\d+$/.test(value)
    leaves.set(path, number ? value : JSON.stringify(value))
    if (stack[stack.length - 1] === 'http_port' && !number) {
      errors.push({ at: path, message: 'expected integer' })
    }
  })
  const hash = [...leaves.entries()].map(([k, v]) => `${k}=${v}`).join(';') + `|${[...hosts.keys()].join(',')}`
  return {
    leaves,
    hosts: [...hosts.entries()].map(([host, groups]) => ({ host, groups: [...groups] })),
    errors,
    hash,
  }
}

function changes(before: Map<string, string>, after: Map<string, string>): InventoryChange[] {
  const out: InventoryChange[] = []
  for (const path of [...new Set([...before.keys(), ...after.keys()])].sort()) {
    const b = before.get(path) ?? null
    const a = after.get(path) ?? null
    if (b === a) continue
    out.push({ path, kind: b === null ? 'added' : a === null ? 'removed' : 'changed', before: b, after: a })
  }
  return out
}

function opError(code: string, message: string): Event[] {
  return [{ type: 'OpError', data: { code, message } }]
}

export class FakeInventories {
  readonly stored: Stored[] = []

  private readonly pending = new Map<string, Pending>()
  private seq = 0

  private next(prefix: string): string {
    return `${prefix}-${String(++this.seq).padStart(4, '0')}`
  }

  /** Adds a managed inventory with one revision per source, oldest first. */
  seed(name: string, sources: string[]): InventoryInfo {
    const info: InventoryInfo = {
      id: this.next('inventory'),
      workspace: WORKSPACE,
      name,
      source: 'managed',
      binding: null,
      path: null,
      status: 'active',
      head_seq: null,
      head_status: null,
      host_count: 0,
      has_schema: true,
      created_at: NOW,
    }
    const s: Stored = { info, revisions: [] }
    this.stored.push(s)
    for (const source of sources) this.append(s, source, null)
    return info
  }

  /** The revisions of `name`, oldest first. */
  revisions(name: string): InventoryRevisionInfo[] {
    return this.stored.find((s) => s.info.name === name)?.revisions ?? []
  }

  private append(s: Stored, source: string, restoredFrom: number | null): void {
    const c = check(source)
    const seq = s.revisions.length + 1
    const summary: InventoryRevisionSummary = {
      seq,
      status: c.errors.length === 0 ? 'valid' : 'invalid',
      content_hash: c.hash,
      source_commit: null,
      author: 'Ada Lovelace',
      restored_from: restoredFrom,
      host_count: c.hosts.length,
      created_at: NOW,
    }
    s.revisions.push({
      summary,
      source,
      content: JSON.stringify(Object.fromEntries(c.leaves), null, 2),
      hosts: c.hosts,
      errors: c.errors,
      warnings: [],
    })
    s.info = { ...s.info, head_seq: seq, head_status: summary.status, host_count: c.hosts.length }
  }

  private find(id: string): Stored | undefined {
    return this.stored.find((s) => s.info.id === id)
  }

  private revision(s: Stored, seq: number | null): Event[] {
    const revision = seq === null ? s.revisions.at(-1) : s.revisions[seq - 1]
    if (!revision) return opError('inventory_not_found', 'No such revision.')
    const history = [...s.revisions].reverse().map((r) => r.summary)
    return [{ type: 'InventoryRevision', data: { inventory: s.info, revision, history } }]
  }

  private preview(action: string, summary: string, affected: string[], apply: () => Event[]): Event[] {
    const preview_id = this.next('preview')
    this.pending.set(preview_id, { apply })
    return [{ type: 'Preview', data: { preview_id, action, summary, affected, step_up: false, expires_at: EXPIRES } }]
  }

  private invalid(c: Checked): Event[] {
    const first = c.errors[0]
    const tail = first ? ` First: ${first.at}: ${first.message}` : ''
    return opError('inventory_invalid', `The inventory has ${String(c.errors.length)} error(s); fix them before saving.${tail}`)
  }

  /** Re-checks the head when a preview is confirmed, as the server does under its row lock. */
  private revise(s: Stored, base: number, source: string, restoredFrom: number | null): Event[] {
    if (s.revisions.length !== base) {
      return opError('stale_base', 'The inventory changed since you started editing. Reload it and try again.')
    }
    this.append(s, source, restoredFrom)
    return this.revision(s, null)
  }

  /** The answer to an inventory command, or `undefined` for any other command. */
  respond(cmd: Cmd): Event[] | undefined {
    switch (cmd.type) {
      case 'ListInventories':
        return [
          {
            type: 'Inventories',
            data: { items: this.stored.filter((s) => s.info.workspace === cmd.data.workspace).map((s) => s.info) },
          },
        ]
      case 'GetInventoryRevision': {
        const s = this.find(cmd.data.inventory)
        return s ? this.revision(s, cmd.data.seq) : opError('inventory_not_found', 'No such inventory.')
      }
      case 'DiffInventory': {
        const s = this.find(cmd.data.inventory)
        const from = s?.revisions[cmd.data.from - 1]
        const to = s?.revisions[cmd.data.to - 1]
        if (!from?.source || !to?.source) return opError('inventory_not_found', 'No such revision.')
        const { inventory } = cmd.data
        return [
          {
            type: 'InventoryDiff',
            data: { inventory, from: cmd.data.from, to: cmd.data.to, changes: changes(check(from.source).leaves, check(to.source).leaves) },
          },
        ]
      }
      case 'ValidateInventory': {
        const c = check(cmd.data.content)
        return [{ type: 'InventoryValidation', data: { content_hash: c.hash, hosts: c.hosts, errors: c.errors, warnings: [] } }]
      }
      case 'PreviewInventoryCreate': {
        const { name, content } = cmd.data
        if (this.stored.some((s) => s.info.name === name)) return opError('name_taken', 'An inventory with that name exists.')
        const c = check(content)
        if (c.errors.length > 0) return this.invalid(c)
        return this.preview('inventory_create', `Create the managed inventory "${name}" with ${String(c.hosts.length)} host(s).`, [name], () => {
          const info = this.seed(name, [content])
          const s = this.find(info.id)
          return s ? this.revision(s, null) : []
        })
      }
      case 'PreviewInventoryEdit': {
        const s = this.find(cmd.data.inventory)
        if (!s) return opError('inventory_not_found', 'No such inventory.')
        const { base_seq, content } = cmd.data
        if (s.revisions.length !== base_seq) return this.revise(s, base_seq, content, null)
        const c = check(content)
        if (c.errors.length > 0) return this.invalid(c)
        if (c.hash === s.revisions.at(-1)?.summary.content_hash) {
          return opError('no_change', 'That is the same content as the newest revision.')
        }
        const summary = `Save revision ${String(base_seq + 1)} of "${s.info.name}" with ${String(c.hosts.length)} host(s).`
        return this.preview('inventory_edit', summary, [s.info.name], () => this.revise(s, base_seq, content, null))
      }
      case 'PreviewInventoryRestore': {
        const s = this.find(cmd.data.inventory)
        const old = s?.revisions[cmd.data.seq - 1]
        if (!s || !old?.source) return opError('inventory_not_found', 'No such revision.')
        const base = s.revisions.length
        if (old.summary.content_hash === s.revisions.at(-1)?.summary.content_hash) {
          return opError('no_change', 'That is the same content as the newest revision.')
        }
        const source = old.source
        const summary = `Restore revision ${String(cmd.data.seq)} of "${s.info.name}" as revision ${String(base + 1)}.`
        return this.preview('inventory_restore', summary, [s.info.name], () => this.revise(s, base, source, cmd.data.seq))
      }
      case 'PreviewInventoryDeprecate': {
        const s = this.find(cmd.data.inventory)
        if (!s) return opError('inventory_not_found', 'No such inventory.')
        const summary = `Deprecate "${s.info.name}". Runs already pinned to it continue; new runs are refused.`
        return this.preview('inventory_deprecate', summary, [s.info.name], () => {
          s.info = { ...s.info, status: 'deprecated' }
          return [{ type: 'Inventories', data: { items: this.stored.map((x) => x.info) } }]
        })
      }
      case 'Confirm': {
        const p = this.pending.get(cmd.data.preview_id)
        if (!p) return undefined
        this.pending.delete(cmd.data.preview_id)
        return [{ type: 'Confirmed', data: { preview_id: cmd.data.preview_id } }, ...p.apply()]
      }
      default:
        return undefined
    }
  }
}
