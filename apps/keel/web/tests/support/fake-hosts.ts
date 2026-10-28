/**
 * The fake backend's hosts (spec 055 US7, T079).
 *
 * An in-memory model answering the host commands as apps/keel/server/src/hosts.rs does, installed
 * on a `FakeBackend` with `on(...)` so the shared fake stays untouched:
 *
 * - `ListHosts` answers `Hosts`; `GetSshCa` answers `SshCa`.
 * - `PreviewHostEnroll` refuses a profile the workspace lacks (`unknown_profile`) or a name already
 *   enrolled (`host_exists`), else previews; its `Confirm` answers `Confirmed` and
 *   `HostEnrollment` with a fresh token, once.
 * - `PreviewHostRevoke` previews with `step_up`; its `Confirm` needs a fresh check (as the shared
 *   fake does), revokes, answers `Confirmed` and `Host`, and pushes `HostState`.
 * - `connect(name)` stands in for the runner running the base profile with the token: the host is
 *   enrolled and pushed online, as `HostState` (the list has not seen it, so it re-lists).
 */
import type { Cmd, Event, HostInfo } from '../../src/protocol/wire'
import type { FakeBackend } from './fake-backend'
import { NOW, WORKSPACE } from './fake-repos'

const EXPIRES = '2026-10-27T12:15:00Z'
export const HOST_CA_FP = 'SHA256:hostcafingerprint0000000000000000000000000'
export const CERT_AUTHORITY = '@cert-authority * ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIHostCa keel-host-ca'

export function hostInfo(over: Partial<HostInfo> = {}): HostInfo {
  return {
    id: 'host-0001',
    workspace: WORKSPACE,
    inventory_name: 'web-1',
    state: 'offline',
    profiles: ['base'],
    allowed_profiles: ['base'],
    require_signed: false,
    runner_version: null,
    ansible_version: null,
    enrolled_at: NOW,
    last_seen_at: null,
    revoked_at: null,
    ...over,
  }
}

type Pending =
  | { kind: 'enroll'; inventory_name: string; profiles: string[] }
  | { kind: 'revoke'; host: string }

export class FakeHosts {
  hosts: HostInfo[] = []
  /** Profiles the workspace declares. */
  profiles = ['base', 'web']
  /** Every token minted, in order (what the hosts would be handed). */
  readonly tokens: { inventory_name: string; profiles: string[]; token: string }[] = []
  private readonly pending = new Map<string, Pending>()
  private seq = 0

  private readonly backend: FakeBackend

  constructor(backend: FakeBackend) {
    this.backend = backend
    for (const type of ['ListHosts', 'GetSshCa', 'PreviewHostEnroll', 'PreviewHostRevoke', 'Confirm'] as const) {
      backend.on(type, (cmd) => this.respond(cmd))
    }
  }

  /** The runner enrolled with the token minted for `name` and connected. */
  connect(name: string): HostInfo {
    const minted = this.tokens.find((t) => t.inventory_name === name)
    const host = hostInfo({
      id: `host-${String(++this.seq).padStart(4, '0')}`,
      inventory_name: name,
      profiles: minted?.profiles ?? ['base'],
      allowed_profiles: minted?.profiles ?? ['base'],
      state: 'online',
      runner_version: '0.1.0',
      ansible_version: '2.17.4',
      last_seen_at: NOW,
    })
    this.hosts = [...this.hosts, host]
    this.backend.push({ type: 'HostState', data: { host: host.id, state: 'online' } })
    return host
  }

  private respond(cmd: Cmd): Event[] | undefined {
    switch (cmd.type) {
      case 'ListHosts':
        return [{ type: 'Hosts', data: { items: this.hosts.filter((h) => h.workspace === cmd.data.workspace) } }]
      case 'GetSshCa':
        return [
          {
            type: 'SshCa',
            data: {
              cas: [
                {
                  id: 'ca-host',
                  kind: 'host',
                  state: 'active',
                  fingerprint: HOST_CA_FP,
                  public_key: 'ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIHostCa keel-host-ca',
                  not_after: null,
                  created_at: NOW,
                },
              ],
              cert_authority: CERT_AUTHORITY,
            },
          },
        ]
      case 'PreviewHostEnroll': {
        const { inventory_name, profiles } = cmd.data
        const missing = profiles.find((p) => !this.profiles.includes(p))
        if (missing !== undefined) {
          return [{ type: 'OpError', data: { code: 'unknown_profile', message: `The workspace has no profile ${missing}.` } }]
        }
        if (this.hosts.some((h) => h.inventory_name === inventory_name)) {
          return [
            {
              type: 'OpError',
              data: { code: 'host_exists', message: `A host named ${inventory_name} is already enrolled in this workspace.` },
            },
          ]
        }
        const preview_id = `host-preview-${String(++this.seq)}`
        this.pending.set(preview_id, { kind: 'enroll', inventory_name, profiles })
        return [
          {
            type: 'Preview',
            data: {
              preview_id,
              action: 'preview_host_enroll',
              summary: `Create a one-time enrolment token for ${inventory_name} with profiles ${profiles.join(', ')}.`,
              affected: [inventory_name],
              step_up: false,
              expires_at: EXPIRES,
            },
          },
        ]
      }
      case 'PreviewHostRevoke': {
        const host = this.hosts.find((h) => h.id === cmd.data.host)
        if (!host) return [{ type: 'OpError', data: { code: 'not_found', message: 'No such host.' } }]
        const preview_id = `host-preview-${String(++this.seq)}`
        this.pending.set(preview_id, { kind: 'revoke', host: host.id })
        return [
          {
            type: 'Preview',
            data: {
              preview_id,
              action: 'preview_host_revoke',
              summary: `Revoke ${host.inventory_name}: its certificates go on the revocation list and its session closes.`,
              affected: [host.inventory_name],
              step_up: true,
              expires_at: EXPIRES,
            },
          },
        ]
      }
      case 'Confirm': {
        const { preview_id, step_up } = cmd.data
        const p = this.pending.get(preview_id)
        if (p === undefined) return undefined
        this.pending.delete(preview_id)
        if (p.kind === 'enroll') {
          const token = `keel-enroll-v1.${String(this.seq).padStart(64, '0')}.${HOST_CA_FP}`
          this.tokens.push({ inventory_name: p.inventory_name, profiles: p.profiles, token })
          return [
            { type: 'Confirmed', data: { preview_id } },
            {
              type: 'HostEnrollment',
              data: {
                id: `enroll-${String(this.seq)}`,
                workspace: WORKSPACE,
                inventory_name: p.inventory_name,
                profiles: p.profiles,
                expires_at: EXPIRES,
                token,
              },
            },
          ]
        }
        if (!step_up && !this.backend.me?.verification.verified) {
          return [{ type: 'OpError', data: { code: 'step_up_required', message: 'Confirm it’s you with a passkey first.' } }]
        }
        let revoked: HostInfo | undefined
        this.hosts = this.hosts.map((h) => {
          if (h.id !== p.host) return h
          revoked = { ...h, state: 'revoked', revoked_at: NOW }
          return revoked
        })
        if (!revoked) return [{ type: 'OpError', data: { code: 'not_found', message: 'No such host.' } }]
        this.backend.push({ type: 'HostState', data: { host: revoked.id, state: 'revoked' } })
        return [
          { type: 'Confirmed', data: { preview_id } },
          {
            type: 'Host',
            data: { host: revoked, python_version: null, override_quorum: null, ledger_head: null, certs: [] },
          },
        ]
      }
      default:
        return undefined
    }
  }
}
