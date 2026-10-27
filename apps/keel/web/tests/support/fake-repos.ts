/**
 * The fake backend's bindings, receivers, deliveries and credentials (spec 055 US1, T043).
 *
 * An in-memory model answering the US1 commands as apps/keel/server does: every `Preview*` answers
 * a `Preview` held until `Confirm`, which answers `Confirmed` and then the changed list. A binding
 * preview "fetches" at `head`; a receiver's secret is answered once, in `ReceiverSecret`.
 * `deliver()` records a delivery and returns the `DeliveryReceived` event to push.
 */
import type {
  BindingInfo,
  Cmd,
  CredentialInfo,
  DeliveryInfo,
  DeliveryRefusal,
  Event,
  JobSummary,
  ReceiverInfo,
} from '../../src/protocol/wire'

export const WORKSPACE = '01J00000000000000000000WS1'
export const PROJECT = '01J00000000000000000000PR1'
export const HEAD = '3f2a9c1d0b7e6a5f4c3b2a1908f7e6d5c4b3a291'
export const NOW = '2026-10-27T12:00:00Z'
const EXPIRES = '2026-10-27T12:10:00Z'

type Pending = { step_up: boolean; apply: () => Event[] }

export class FakeRepos {
  bindings: BindingInfo[] = []
  jobs = new Map<string, JobSummary[]>()
  receivers: ReceiverInfo[] = []
  deliveries: DeliveryInfo[] = []
  credentials: CredentialInfo[] = []
  /** Secrets handed out, so a spec can check none is shown twice. */
  readonly secrets: string[] = []
  /** Confirms that carried a passkey assertion. */
  stepUps = 0

  private readonly pending = new Map<string, Pending>()
  private seq = 0

  private next(prefix: string): string {
    return `${prefix}-${String(++this.seq).padStart(4, '0')}`
  }

  private preview(action: string, summary: string, affected: string[], step_up: boolean, apply: () => Event[]): Event[] {
    const preview_id = this.next('preview')
    this.pending.set(preview_id, { step_up, apply })
    return [{ type: 'Preview', data: { preview_id, action, summary, affected, step_up, expires_at: EXPIRES } }]
  }

  private binding(id: string): Event {
    const binding = this.bindings.find((b) => b.id === id)
    if (!binding) return { type: 'OpError', data: { code: 'not_found', message: 'No such binding.' } }
    return { type: 'Binding', data: { binding, jobs: this.jobs.get(id) ?? [] } }
  }

  private receiverList(): Event {
    return { type: 'Receivers', data: { items: this.receivers } }
  }

  /** Records a delivery for `receiver` and returns the event the server pushes. */
  deliver(receiver: string, providerId: string, refusal: DeliveryRefusal | null = null): Event {
    const r = this.receivers.find((x) => x.id === receiver)
    const binding = refusal === null ? (r?.bindings[0] ?? null) : null
    const delivery: DeliveryInfo = {
      id: this.next('delivery'),
      receiver,
      binding,
      provider_delivery_id: providerId,
      received_at: NOW,
      decision: refusal === null ? 'accepted' : 'refused',
      refusal,
      sync_state: refusal === null ? 'synced' : null,
      commit: refusal === null ? HEAD : null,
    }
    this.deliveries = [delivery, ...this.deliveries]
    return { type: 'DeliveryReceived', data: { delivery } }
  }

  /** The answer to a US1 command, or `undefined` for any other command. */
  respond(cmd: Cmd): Event[] | undefined {
    switch (cmd.type) {
      case 'ListBindings':
        return [{ type: 'Bindings', data: { items: this.bindings } }]
      case 'GetBinding':
        return [this.binding(cmd.data.binding)]
      case 'SyncBinding':
        return [{ type: 'BindingSynced', data: { binding: cmd.data.binding, commit: HEAD, jobs_changed: false, inventories_changed: false } }]
      case 'PreviewBindingCreate': {
        const spec = cmd.data
        if (this.bindings.some((b) => b.url === spec.url && b.branch === spec.branch)) {
          return [{ type: 'OpError', data: { code: 'binding_exists', message: 'That repository and branch are already bound.' } }]
        }
        const summary = `Bind ${spec.url} @ ${spec.branch}: fetched ${HEAD.slice(0, 12)}, 1 job (web).`
        return this.preview('binding_create', summary, [spec.url], false, () => {
          const binding: BindingInfo = {
            id: this.next('binding'),
            project: spec.project,
            workspace: WORKSPACE,
            url: spec.url,
            branch: spec.branch,
            transport: spec.transport,
            credential: spec.credential,
            known_host: spec.known_host,
            poll_interval: spec.poll_interval,
            allow_force_push: spec.allow_force_push,
            signed_mode: spec.signed_mode,
            override_quorum: spec.override_quorum,
            status: 'active',
            last_commit: HEAD,
            last_synced_at: NOW,
            last_sync_error: null,
            manifest_problems: [],
          }
          this.bindings.push(binding)
          this.jobs.set(binding.id, [
            { name: 'web', playbook: 'playbooks/web.yaml', profiles: ['debian'], inventory: 'prod', blocked: null, defined_at_commit: HEAD },
          ])
          return [this.binding(binding.id)]
        })
      }
      case 'ListReceivers':
        return [this.receiverList()]
      case 'PreviewReceiverCreate': {
        const { workspace, name, provider } = cmd.data
        return this.preview('receiver_create', `Create receiver ${name} (${provider}).`, [name], true, () => {
          const id = this.next('receiver')
          const url = `https://keel.example.com/hooks/${id}`
          const secret = `whsec-${id}-not-shown-again`
          this.secrets.push(secret)
          this.receivers.push({
            id,
            workspace,
            name,
            provider,
            status: 'active',
            url,
            previous_expires_at: null,
            bindings: [],
            created_at: NOW,
            rotated_at: null,
          })
          return [{ type: 'ReceiverSecret', data: { url, secret } }, this.receiverList()]
        })
      }
      case 'PreviewReceiverBind': {
        const { receiver, binding } = cmd.data
        const b = this.bindings.find((x) => x.id === binding)
        const r = this.receivers.find((x) => x.id === receiver)
        if (!b || !r) return [{ type: 'OpError', data: { code: 'not_found', message: 'No such receiver or binding.' } }]
        return this.preview('receiver_bind', `Let ${r.name} sync ${b.url} @ ${b.branch}.`, [b.url], true, () => {
          r.bindings = [...r.bindings, binding]
          return [this.receiverList()]
        })
      }
      case 'ListDeliveries':
        return [{ type: 'Deliveries', data: { items: this.deliveries } }]
      case 'ListCredentials':
        return [{ type: 'Credentials', data: { items: this.credentials } }]
      case 'Confirm': {
        const p = this.pending.get(cmd.data.preview_id)
        if (!p) return [{ type: 'OpError', data: { code: 'preview_expired', message: 'That preview expired.' } }]
        this.pending.delete(cmd.data.preview_id)
        if (cmd.data.step_up) this.stepUps++
        return [{ type: 'Confirmed', data: { preview_id: cmd.data.preview_id } }, ...p.apply()]
      }
      default:
        return undefined
    }
  }
}
