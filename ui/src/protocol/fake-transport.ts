/**
 * In-page scripted backend for `pnpm dev:fake` and Playwright (contracts/ipc.md). It records
 * every command, replays a named scenario's replies from `ui/tests/fixtures/`, and exposes
 * `window.__fathomFake` so E2E tests can inject batches and assert the emitted commands.
 *
 * Only loaded when `VITE_TRANSPORT=fake` (see transport.ts), so none of this ships in the app.
 */
import { FIXTURES, type FixtureName } from '../../tests/fixtures'
import {
  isScenarioName,
  SCENARIOS,
  type FakeBackendState,
  type Reply,
  type Scenario,
  type ScenarioName,
} from '../../tests/fixtures/scenarios'
import type { Transport } from './transport'
import type { BackendCmd, EventBatch, SubscriptionInfo } from './types'

/** The Playwright-facing handle (`window.__fathomFake`). */
export interface FathomFake {
  /** Deliver a batch (or a named fixture from ui/tests/fixtures) to the UI. */
  emit(batch: EventBatch | FixtureName): void
  /** Every command the UI sent, oldest first (a copy). */
  sent(): BackendCmd[]
  clearSent(): void
  /** Switch scenario: resets backend state and emits the scenario's on-subscribe batches. */
  load(name: ScenarioName): void
  /** The active scenario's name. */
  scenario(): ScenarioName
  scenarios(): ScenarioName[]
  /** While true, `send` rejects with `BridgeError::BackendGone` (the backend stopped). */
  setBackendGone(gone: boolean): void
  /** Delay before auto-replies are delivered, in ms (default 0). */
  setLatency(ms: number): void
  /** The `RequestId` the next `send` resolves to (ids increment from 1). */
  nextRequestId(): number
  /**
   * Deliver the `CommandFinished` for `requestId` (correlated with it, system route), as
   * the real backend does once the command's handler completes.
   */
  finish(requestId: number, ok: boolean, error?: string): void
  /** How many times the UI asked to sign out (hosted mode). */
  signOuts(): number
}

declare global {
  interface Window {
    __fathomFake?: FathomFake
  }
}

export interface FakeTransportOptions {
  scenario?: ScenarioName
  latencyMs?: number
}

export class FakeTransport implements Transport {
  #scenarioName: ScenarioName
  #scenario: Scenario
  #state: FakeBackendState
  #sent: BackendCmd[] = []
  #onBatch: ((batch: EventBatch) => void) | undefined
  #gone = false
  #latencyMs: number
  #nextRequestId = 1
  #finishSeq = 1_000_000

  constructor(options: FakeTransportOptions = {}) {
    this.#scenarioName = options.scenario ?? 'default'
    this.#scenario = SCENARIOS[this.#scenarioName]
    this.#state = structuredClone(this.#scenario.initialState)
    this.#latencyMs = options.latencyMs ?? 0
  }

  send(cmd: BackendCmd): Promise<number> {
    this.#sent.push(cmd)
    if (this.#gone) return Promise.reject(new BackendGoneError())
    const id = this.#nextRequestId++
    const reply = this.#scenario.replies[cmd.type] as Reply | undefined
    if (reply) {
      const batches = reply(cmd, this.#state)
      this.#deliverLater(batches)
    }
    return Promise.resolve(id)
  }

  get nextRequestId(): number {
    return this.#nextRequestId
  }

  /** Emit `CommandFinished` for `requestId` immediately. */
  finish(requestId: number, ok: boolean, error?: string): void {
    this.emit({
      envelopes: [
        {
          version: 1,
          seq: this.#finishSeq++,
          route: { section: 'system', scope: 'Global', context: null, namespace: null },
          correlation: { type: 'Request', data: requestId },
          event: { type: 'CommandFinished', data: { ok, error: error ?? null } },
        },
      ],
      dropped: 0,
    })
  }

  subscribe(onBatch: (batch: EventBatch) => void): Promise<SubscriptionInfo> {
    if (this.#gone) return Promise.reject(new BackendGoneError())
    this.#onBatch = onBatch
    this.#deliverLater(this.#scenario.onSubscribe)
    return Promise.resolve(subscriptionInfoFromUrl())
  }

  #signOuts = 0
  get signOuts(): number {
    return this.#signOuts
  }

  /** Stand-in for `HostedTransport.signOut()` (hosted-mode specs). */
  signOut(): Promise<string | null> {
    this.#signOuts += 1
    return Promise.resolve(null)
  }

  /** Deliver immediately (tests); batches sent before `subscribe` are lost, like the pump's. */
  emit(batch: EventBatch): void {
    this.#onBatch?.(batch)
  }

  sent(): BackendCmd[] {
    return [...this.#sent]
  }

  clearSent(): void {
    this.#sent = []
  }

  load(name: ScenarioName): void {
    this.#scenarioName = name
    this.#scenario = SCENARIOS[name]
    this.#state = structuredClone(this.#scenario.initialState)
    this.#deliverLater(this.#scenario.onSubscribe)
  }

  get scenarioName(): ScenarioName {
    return this.#scenarioName
  }

  setBackendGone(gone: boolean): void {
    this.#gone = gone
  }

  setLatency(ms: number): void {
    this.#latencyMs = ms
  }

  #deliverLater(batches: readonly EventBatch[]): void {
    if (batches.length === 0) return
    // Async like the real bridge: replies never arrive inside the `send` call.
    setTimeout(() => {
      for (const batch of batches) this.#onBatch?.(batch)
    }, this.#latencyMs)
  }
}

/** Shape of the rejection Tauri's `invoke` produces for `BridgeError::BackendGone`. */
export class BackendGoneError extends Error {
  readonly type = 'BackendGone'
  constructor() {
    super('BackendGone')
    this.name = 'BackendGoneError'
  }
}

/** `?scenario=<name>` in the page URL, if it names a known scenario. */
export function scenarioFromUrl(
  search: string = globalThis.location.search,
): ScenarioName | undefined {
  const name = new URLSearchParams(search).get('scenario')
  if (name === null) return undefined
  if (isScenarioName(name)) return name
  console.warn(`[fake] unknown scenario "${name}"; using "default"`)
  return undefined
}

/** Create the app's fake transport and expose it as `window.__fathomFake`. */
/**
 * `?mode=hosted&user=alice&groups=a,b&limits=2,3,4` makes the fake present as a hosted backend
 * (031-hosted-server-mode); without `mode` it is the desktop shell, as before.
 */
function subscriptionInfoFromUrl(): SubscriptionInfo {
  const info: SubscriptionInfo = { protocolVersion: 1, backendStarted: true }
  const q = new URLSearchParams(globalThis.location.search)
  if (q.get('mode') !== 'hosted') return info
  info.mode = 'hosted'
  const user = q.get('user')
  if (user !== null) {
    info.identity = { username: user, groups: (q.get('groups') ?? '').split(',').filter(Boolean) }
  }
  const [w, st, t] = (q.get('limits') ?? '100,20,10').split(',').map(Number)
  info.limits = { maxWatches: w ?? 100, maxStreams: st ?? 20, maxTabs: t ?? 10 }
  return info
}

export function installFakeTransport(): FakeTransport {
  const fake = new FakeTransport({ scenario: scenarioFromUrl() })
  window.__fathomFake = {
    emit: (batch) => fake.emit(typeof batch === 'string' ? FIXTURES[batch] : batch),
    sent: () => fake.sent(),
    clearSent: () => fake.clearSent(),
    load: (name) => fake.load(name),
    scenario: () => fake.scenarioName,
    scenarios: () => Object.keys(SCENARIOS) as ScenarioName[],
    setBackendGone: (gone) => fake.setBackendGone(gone),
    setLatency: (ms) => fake.setLatency(ms),
    nextRequestId: () => fake.nextRequestId,
    finish: (requestId, ok, error) => fake.finish(requestId, ok, error),
    signOuts: () => fake.signOuts,
  }
  return fake
}
