/**
 * Spec 054 (T068): passkeys and step-up in the fake backend, off until a story turns them on.
 *
 * Off, the fake answers as a server without the gate: nothing required, no passkeys, every
 * ceremony refused. On (`enabled`):
 *
 * - `Verification` says what the session has: an admin must verify (`required`), `enrolled` follows
 *   `items`. An admin who has not verified gets `Denied{not_permitted}` for admin actions.
 * - Ceremonies are real enough for Chromium's virtual authenticator (`./webauthn.ts`): the options
 *   name `localhost`, and a finish is accepted when its credential id is one of `items`. Signatures
 *   are not checked.
 * - While `stale` (the last check is older than the window), a preview carries `step_up` and its
 *   `Confirm` needs the assertion; `OpenShell` answers `StepUpRequired` (and accepts an assertion
 *   on the retry); `PutUser`, `RemoveUser` and `PreviewFileOp` answer `StepUpRequired` until a
 *   `PasskeyFinish` makes the session fresh. Someone with no passkey gets `PasskeyEnrolmentRequired`.
 * - `refuseNext` makes the next check fail with that reason, once.
 */
import type {
  Cmd,
  Event,
  Me,
  PasskeyAssertion,
  PasskeyInfo,
  PasskeyRefusedReason,
  PasskeyRequest,
  Preview,
  Verification,
} from '../../src/protocol/generated'

const RP_ID = 'localhost'
const NOW = '2026-10-28T12:00:00Z'

/** The commands an unverified admin is refused, and that need a fresh check when stale. */
const ADMIN_ACTIONS: ReadonlySet<Cmd['type']> = new Set<Cmd['type']>([
  'PreviewOp',
  'PreviewSave',
  'RevealEnv',
  'OpenShell',
  'PutUser',
  'RemoveUser',
  'PreviewFileOp',
  'PreviewUpdateChange',
  'PreviewUpdateBatch',
])
/** Commands with no assertion of their own: a stale session finishes a ceremony first. */
const FRESH_FIRST: ReadonlySet<Cmd['type']> = new Set<Cmd['type']>(['PutUser', 'RemoveUser', 'PreviewFileOp'])

const b64url = (s: string): string => Buffer.from(s).toString('base64url')

type Ceremony = { kind: 'register'; name: string } | { kind: 'verify' }

export class FakePasskeys {
  enabled = false
  items: PasskeyInfo[] = []
  verified = false
  recovering = false
  /** The session's last check is out of the window: risky actions ask for a fresh one. */
  stale = false
  recoveryCode = 'RECOVERY1234'
  /** The next check (finish or assertion) is refused with this, once. */
  refuseNext: PasskeyRefusedReason | null = null

  private readonly me: () => Me | null
  private readonly nextId: () => string
  private readonly ceremonies = new Map<string, Ceremony>()
  /** Previews that asked for a step-up. */
  private readonly guarded = new Set<string>()
  /** Removal previews, by id, to the passkey they remove. */
  private readonly removals = new Map<string, string>()

  constructor(me: () => Me | null, nextId: () => string) {
    this.me = me
    this.nextId = nextId
  }

  /** Turns the gate on, with these passkeys (ids as `./webauthn.ts` seeds them). */
  enable(over: Partial<Pick<FakePasskeys, 'items' | 'verified' | 'stale' | 'recovering'>> = {}): this {
    this.enabled = true
    Object.assign(this, over)
    return this
  }

  verification(): Verification {
    const me = this.me()
    return {
      verified: this.enabled && this.verified,
      at: this.enabled && this.verified ? NOW : null,
      window_secs: 300,
      enrolled: this.items.length > 0,
      required: this.enabled && me?.role === 'admin',
      recovering: this.recovering,
    }
  }

  /** A verify ceremony, as `StepUpRequired` and `Preview.step_up` carry. */
  request(): PasskeyRequest {
    const ceremony = `cer-${this.nextId()}`
    this.ceremonies.set(ceremony, { kind: 'verify' })
    return {
      ceremony,
      options: {
        publicKey: {
          challenge: b64url(`challenge-${ceremony}`),
          rpId: RP_ID,
          userVerification: 'required',
          allowCredentials: this.items.map((p) => ({ type: 'public-key', id: p.id })),
          timeout: 60_000,
        },
      },
    }
  }

  /** Checks an assertion; a good one makes the session fresh. */
  private check(ceremony: string, credentialId: string): PasskeyRefusedReason | null {
    const c = this.ceremonies.get(ceremony)
    this.ceremonies.delete(ceremony)
    if (this.refuseNext) {
      const r = this.refuseNext
      this.refuseNext = null
      return r
    }
    if (c?.kind !== 'verify') return 'expired'
    const p = this.items.find((x) => x.id === credentialId)
    if (!p) return 'unknown_credential'
    p.last_used = NOW
    this.verified = true
    this.stale = false
    return null
  }

  private assert(a: PasskeyAssertion): PasskeyRefusedReason | null {
    return this.check(a.ceremony, a.credential.id)
  }

  /** Answers the command before its usual handling, or `null` to go on. */
  gate(cmd: Cmd): Event[] | null {
    if (!this.enabled || !ADMIN_ACTIONS.has(cmd.type)) return null
    const me = this.me()
    if (me?.role === 'admin' && !this.verified) {
      return [{ type: 'Denied', data: { reason: 'not_permitted', detail: 'Confirm it\'s you with a passkey first.' } }]
    }
    if (!this.stale || !(FRESH_FIRST.has(cmd.type) || cmd.type === 'OpenShell')) return null
    if (this.items.length === 0) return [{ type: 'PasskeyEnrolmentRequired' }]
    if (cmd.type === 'OpenShell' && cmd.data.assertion) {
      const refused = this.assert(cmd.data.assertion)
      return refused ? [{ type: 'PasskeyRefused', data: { reason: refused } }] : null
    }
    return [{ type: 'StepUpRequired', data: { request: this.request() } }]
  }

  /** Marks a preview as needing a step-up while the session is stale. */
  stamp(preview: Preview): Preview {
    if (this.enabled && this.stale && this.items.length > 0) {
      preview.step_up = this.request()
      this.guarded.add(preview.preview_id)
    }
    return preview
  }

  /** `stamp` for previews that are not a `Preview` (062's `UpdatePreview`): the request, if one is due. */
  stampId(previewId: string): PasskeyRequest | undefined {
    if (!(this.enabled && this.stale && this.items.length > 0)) return undefined
    this.guarded.add(previewId)
    return this.request()
  }

  /** A `Confirm` of a guarded or removal preview, answered here; `null` to go on. */
  confirm(cmd: Extract<Cmd, { type: 'Confirm' }>): Event[] | null {
    const { preview_id, assertion } = cmd.data
    if (this.guarded.has(preview_id)) {
      if (assertion) {
        const refused = this.assert(assertion)
        if (refused) return [{ type: 'PasskeyRefused', data: { reason: refused } }]
      } else if (this.stale) {
        return [{ type: 'StepUpRequired', data: { request: this.request() } }]
      }
      this.guarded.delete(preview_id)
    }
    const id = this.removals.get(preview_id)
    if (id === undefined) return null
    this.removals.delete(preview_id)
    if (this.items.length === 1) return [{ type: 'PasskeyRefused', data: { reason: 'last_passkey' } }]
    this.items = this.items.filter((p) => p.id !== id)
    return [{ type: 'Passkeys', data: { items: this.items } }]
  }

  /** The passkey commands themselves. */
  respond(cmd: Cmd): Event[] | null {
    const me = this.me()
    switch (cmd.type) {
      case 'GetVerification':
        return [{ type: 'Verification', data: this.verification() }]

      case 'ListPasskeys':
        return [{ type: 'Passkeys', data: { items: this.enabled ? this.items : [] } }]

      case 'PasskeyBegin': {
        if (!this.enabled || !me) return [{ type: 'PasskeyRefused', data: { reason: 'invalid' } }]
        const { purpose } = cmd.data
        if (purpose.kind === 'verify') return [{ type: 'PasskeyChallenge', data: this.request() }]
        // Adding a second passkey needs the first; the first (or one after recovery) does not.
        if (this.items.length > 0 && !this.recovering && (!this.verified || this.stale)) {
          return [{ type: 'StepUpRequired', data: { request: this.request() } }]
        }
        const ceremony = `cer-${this.nextId()}`
        this.ceremonies.set(ceremony, { kind: 'register', name: purpose.name })
        return [
          {
            type: 'PasskeyChallenge',
            data: {
              ceremony,
              options: {
                publicKey: {
                  rp: { id: RP_ID, name: 'Depths' },
                  user: { id: b64url(me.identity.subject), name: me.identity.username, displayName: me.identity.username },
                  challenge: b64url(`challenge-${ceremony}`),
                  pubKeyCredParams: [{ type: 'public-key', alg: -7 }],
                  authenticatorSelection: { residentKey: 'required', userVerification: 'required' },
                  excludeCredentials: this.items.map((p) => ({ type: 'public-key', id: p.id })),
                  timeout: 60_000,
                },
              },
            },
          },
        ]
      }

      case 'PasskeyFinish': {
        const c = this.ceremonies.get(cmd.data.ceremony)
        if (c?.kind === 'register') {
          this.ceremonies.delete(cmd.data.ceremony)
          const info: PasskeyInfo = { id: cmd.data.credential.id, name: c.name, created: NOW, last_used: null }
          this.items = [...this.items, info]
          this.verified = true
          this.stale = false
          this.recovering = false
          return [{ type: 'PasskeyRegistered', data: info }]
        }
        const refused = this.check(cmd.data.ceremony, cmd.data.credential.id)
        return refused ? [{ type: 'PasskeyRefused', data: { reason: refused } }] : [{ type: 'Verification', data: this.verification() }]
      }

      case 'PreviewPasskeyRemove': {
        const target = this.items.find((p) => p.id === cmd.data.id)
        if (!this.enabled || !target) return [{ type: 'PasskeyRefused', data: { reason: 'unknown_credential' } }]
        if (this.items.length === 1) return [{ type: 'PasskeyRefused', data: { reason: 'last_passkey' } }]
        const preview: Preview = {
          preview_id: `pv-${this.nextId()}`,
          stack: '',
          kind: 'delete',
          steps: [],
          file_diff: null,
          reaches: { containers: [], networks: [], volumes: [] },
          destructive: true,
          stack_hash: '',
          expires_at: '2026-10-28T12:05:00Z',
        }
        this.removals.set(preview.preview_id, target.id)
        return [{ type: 'Preview', data: this.stamp(preview) }]
      }

      case 'RecoverPasskeys':
        if (!this.enabled || cmd.data.code !== this.recoveryCode) return [{ type: 'PasskeyRefused', data: { reason: 'recovery_invalid' } }]
        this.recovering = true
        this.verified = false
        return [{ type: 'Verification', data: this.verification() }]

      default:
        return null
    }
  }
}
