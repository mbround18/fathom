/**
 * System Settings on the fake backend (spec 060 US2, US3, US7): the instance settings, every
 * enrolment token, every API token, and the System workspace, answered as
 * apps/keel/server/src/{settings,enrollments,system}.rs do.
 *
 * - `GetSettings` answers `Settings`. `PreviewSettingsUpdate` refuses an invalid template with its
 *   code (the twin in src/features/settings/template.ts) or long instructions
 *   (`instructions_too_long`), else previews with step-up; its `Confirm` answers
 *   `settings_conflict` when the version moved (`savedElsewhere()`), else `Settings`.
 * - `ListEnrollments` filters by state. `PreviewEnrollmentRevoke` takes only a pending token
 *   (`enrollment_not_pending`); its `Confirm` answers `Enrollments`. `PreviewEnrollmentRoll` takes
 *   any token not used; its `Confirm` revokes the old one and answers `HostEnrollment`.
 * - `ListAllApiTokens` filters by state; `PreviewApiTokenRevoke` revokes any active token.
 * - `GetSystem` answers `System`. `PreviewSystemConfigure` creates or changes it (with
 *   `system_conflict` on a moved version); `PreviewSystemRemove` removes it.
 *
 * Every change previews with `step_up`; the `Confirm` needs the session's check to be fresh, as the
 * shared fake does. It is chained, so other fakes' previews still confirm.
 */
import { validate } from '../../src/features/settings/template'
import type {
  AdminApiTokenInfo,
  Cmd,
  EnrollmentInfo,
  Event,
  RoleMappingInfo,
} from '../../src/protocol/wire'
import type { FakeBackend } from './fake-backend'
import { NOW, WORKSPACE } from './fake-repos'

const EXPIRES = '2026-10-27T12:15:00Z'
const LATER = '2026-10-27T12:15:00Z'
export const DEFAULT_TEMPLATE = 'read -rs KEEL_TOKEN; export KEEL_TOKEN  # paste: %t\nmake keel-node'
export const SYSTEM_WORKSPACE = '01J0000000000000000000SYS'

type Settings = Extract<Event, { type: 'Settings' }>['data']
type System = Extract<Event, { type: 'System' }>['data']

type Pending =
  | { kind: 'settings'; template: string; instructions: string; version: number }
  | { kind: 'enrollment_revoke'; id: string }
  | { kind: 'enrollment_roll'; id: string }
  | { kind: 'api_token_revoke'; id: string }
  | { kind: 'system_configure'; allowed_profiles: string[]; version: number }
  | { kind: 'system_remove' }

export function enrollment(over: Partial<EnrollmentInfo> = {}): EnrollmentInfo {
  return {
    id: 'enroll-0001',
    workspace: WORKSPACE,
    workspace_slug: 'ops',
    inventory_name: 'web-1',
    profiles: ['base'],
    state: 'pending',
    created_by: '01J0000000000000000000ADA',
    created_at: NOW,
    expires_at: LATER,
    host: null,
    rolled_from: null,
    ...over,
  }
}

export function adminToken(over: { id: string; name: string; holder_name: string } & Partial<AdminApiTokenInfo>): AdminApiTokenInfo {
  const { id, name, holder_name, ...rest } = over
  return {
    token: {
      id,
      name,
      owner: `user-${holder_name}`,
      workspace: WORKSPACE,
      workspace_slug: 'ops',
      actions: ['list_runs'],
      expires_at: '2026-11-27T12:00:00Z',
      last_used_at: null,
      revoked: false,
    },
    holder: `user-${holder_name}`,
    holder_name,
    workspace_slug: 'ops',
    state: 'active',
    ...rest,
  }
}

export class FakeSettings {
  template: string | null = null
  instructions = ''
  version = 0
  runner_address: string | null = 'keel.example.com:7443'
  role_mapping: RoleMappingInfo | null = null
  warnings: string[] = []
  enrollments: EnrollmentInfo[] = []
  apiTokens: AdminApiTokenInfo[] = []
  system: System = { configured: false, workspace: null, allowed_profiles: [], version: 0, bindings: [], hosts: [] }
  /** Tokens minted by a roll, in order. */
  readonly minted: string[] = []

  private readonly pending = new Map<string, Pending>()
  private seq = 0
  private readonly backend: FakeBackend

  constructor(backend: FakeBackend) {
    this.backend = backend
    for (const type of [
      'GetSettings',
      'PreviewSettingsUpdate',
      'ListEnrollments',
      'PreviewEnrollmentRevoke',
      'PreviewEnrollmentRoll',
      'ListAllApiTokens',
      'PreviewApiTokenRevoke',
      'GetSystem',
      'PreviewSystemConfigure',
      'PreviewSystemRemove',
    ] as const) {
      backend.on(type, (cmd) => this.respond(cmd))
    }
    backend.chain('Confirm', (cmd) => this.respond(cmd))
  }

  /** Someone else saved the settings: the version moves on and the template changes. */
  savedElsewhere(template = 'echo someone else %t'): void {
    this.template = template
    this.version++
  }

  /** Someone else changed the System workspace's profiles. */
  systemChangedElsewhere(): void {
    this.system = { ...this.system, version: this.system.version + 1 }
  }

  settings(): Settings {
    return {
      template: this.template ?? DEFAULT_TEMPLATE,
      is_default: this.template === null,
      instructions: this.instructions,
      version: this.version,
      runner_address: this.runner_address,
      public_url: 'https://keel.example.com',
      role_mapping: this.role_mapping,
      warnings: this.warnings,
    }
  }

  private preview(pending: Pending, action: string, summary: string, affected: string[] = []): Event[] {
    const preview_id = `settings-preview-${String(++this.seq)}`
    this.pending.set(preview_id, pending)
    return [{ type: 'Preview', data: { preview_id, action, summary, affected, step_up: true, expires_at: EXPIRES } }]
  }

  private error(code: string, message: string): Event[] {
    return [{ type: 'OpError', data: { code, message } }]
  }

  private respond(cmd: Cmd): Event[] | undefined {
    switch (cmd.type) {
      case 'GetSettings':
        return [{ type: 'Settings', data: this.settings() }]
      case 'PreviewSettingsUpdate': {
        const { template, instructions, version } = cmd.data
        const invalid = validate(template)
        if (invalid) return this.error(invalid.code, `The template is invalid (${invalid.code}).`)
        if (new TextEncoder().encode(instructions).length > 20 * 1024) {
          return this.error('instructions_too_long', 'The instructions are longer than 20480 bytes.')
        }
        return this.preview(
          { kind: 'settings', template, instructions, version },
          'settings_update',
          'Save the enrolment snippet template and instructions. Tokens minted from now on use them.',
        )
      }
      case 'ListEnrollments': {
        const { state } = cmd.data
        return [{ type: 'Enrollments', data: { items: this.enrollments.filter((e) => state === null || e.state === state) } }]
      }
      case 'PreviewEnrollmentRevoke': {
        const e = this.enrollments.find((x) => x.id === cmd.data.enrollment)
        if (!e) return this.error('not_found', 'No such enrolment token.')
        if (e.state !== 'pending') return this.error('enrollment_not_pending', 'Only a pending token can be revoked.')
        return this.preview({ kind: 'enrollment_revoke', id: e.id }, 'enrollment_revoke', `Revoke the enrolment token for ${e.inventory_name}.`, [
          e.inventory_name,
        ])
      }
      case 'PreviewEnrollmentRoll': {
        const e = this.enrollments.find((x) => x.id === cmd.data.enrollment)
        if (!e) return this.error('not_found', 'No such enrolment token.')
        if (e.state === 'used') return this.error('enrollment_used', 'That token was used; enroll the host again instead.')
        return this.preview(
          { kind: 'enrollment_roll', id: e.id },
          'enrollment_roll',
          `Roll the enrolment token for ${e.inventory_name}: the old one stops working and a new one is shown once.`,
          [e.inventory_name],
        )
      }
      case 'ListAllApiTokens': {
        const { state } = cmd.data
        return [{ type: 'AllApiTokens', data: { items: this.apiTokens.filter((t) => state === null || t.state === state) } }]
      }
      case 'PreviewApiTokenRevoke': {
        const t = this.apiTokens.find((x) => x.token.id === cmd.data.token)
        if (!t) return this.error('not_found', 'No such API token.')
        return this.preview({ kind: 'api_token_revoke', id: t.token.id }, 'api_token_revoke', `Revoke API token ${t.token.name} of ${t.holder_name}.`)
      }
      case 'GetSystem':
        return [{ type: 'System', data: this.system }]
      case 'PreviewSystemConfigure': {
        const { allowed_profiles, version } = cmd.data
        return this.preview(
          { kind: 'system_configure', allowed_profiles, version },
          'system_configure',
          this.system.configured
            ? `Allow profiles ${allowed_profiles.join(', ')} in keel-system.`
            : `Create keel-system, allowing ${allowed_profiles.join(', ')}.`,
        )
      }
      case 'PreviewSystemRemove':
        return this.preview({ kind: 'system_remove' }, 'system_remove', 'Remove keel-system: revoke its hosts, disable its bindings, archive it.')
      case 'Confirm': {
        const { preview_id, step_up } = cmd.data
        const p = this.pending.get(preview_id)
        if (p === undefined) return undefined
        this.pending.delete(preview_id)
        if (!step_up && !this.backend.me?.verification.verified) {
          return this.error('step_up_required', 'Confirm it’s you with a passkey first.')
        }
        // A conflict answers instead of `Confirmed`: nothing was saved.
        const applied = this.apply(p)
        if (applied[0]?.type === 'OpError') return applied
        return [{ type: 'Confirmed', data: { preview_id } }, ...applied]
      }
      default:
        return undefined
    }
  }

  private apply(p: Pending): Event[] {
    switch (p.kind) {
      case 'settings':
        if (p.version !== this.version) {
          return this.error('settings_conflict', 'The settings changed since you loaded them. Reload and try again.')
        }
        this.template = p.template
        this.instructions = p.instructions
        this.version++
        return [{ type: 'Settings', data: this.settings() }]
      case 'enrollment_revoke':
        this.enrollments = this.enrollments.map((e) => (e.id === p.id ? { ...e, state: 'revoked' } : e))
        return [{ type: 'Enrollments', data: { items: this.enrollments } }]
      case 'enrollment_roll': {
        const old = this.enrollments.find((e) => e.id === p.id)
        if (!old) return this.error('not_found', 'No such enrolment token.')
        const id = `enroll-${String(++this.seq).padStart(4, '0')}`
        const token = `keel-enroll-v1.${String(this.seq).padStart(64, '0')}.SHA256:rolled`
        this.minted.push(token)
        const expires_at = new Date(Date.now() + 15 * 60_000).toISOString()
        this.enrollments = [
          enrollment({ ...old, id, state: 'pending', rolled_from: old.id, expires_at }),
          ...this.enrollments.map((e) => (e.id === p.id ? { ...e, state: 'revoked' as const } : e)),
        ]
        return [
          {
            type: 'HostEnrollment',
            data: {
              id,
              workspace: old.workspace,
              inventory_name: old.inventory_name,
              profiles: old.profiles,
              expires_at,
              token,
              snippet: `read -rs KEEL_TOKEN; export KEEL_TOKEN  # paste: ${token}\nmake keel-node`,
              instructions: this.instructions,
              instructions_unavailable: false,
            },
          },
        ]
      }
      case 'api_token_revoke':
        this.apiTokens = this.apiTokens.map((t) =>
          t.token.id === p.id ? { ...t, state: 'revoked', token: { ...t.token, revoked: true } } : t,
        )
        return []
      case 'system_configure':
        if (p.version !== this.system.version) {
          return this.error('system_conflict', 'The System workspace changed since you loaded it. Reload and try again.')
        }
        this.system = {
          ...this.system,
          configured: true,
          workspace: SYSTEM_WORKSPACE,
          allowed_profiles: p.allowed_profiles,
          version: this.system.version + 1,
        }
        return [{ type: 'System', data: this.system }]
      case 'system_remove':
        this.system = { configured: false, workspace: null, allowed_profiles: [], version: 0, bindings: [], hosts: [] }
        return [{ type: 'System', data: this.system }]
    }
  }
}
