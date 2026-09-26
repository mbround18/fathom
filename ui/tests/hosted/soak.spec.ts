/**
 * Load / soak (spec 031 T054, SC-006): N real signed-in users (provider accounts soak-01..N in
 * fathom-viewers, created through the provider admin API), each with its own browser context and
 * a `/ws` connection, browsing and watching for SOAK_MINUTES. Asserts, per user:
 *   - `subscribed.info.identity` is that user;
 *   - no frame ever carries another user's marker (each user reads a unique, non-existent
 *     deployment `soak-<i>-marker`; the NotFound text names it) and no CommandFinished for a
 *     request id the user did not send (no cross-user events);
 *   - no bridge `error` frame, no socket close, every command finishes;
 * and samples the fathom-server process (RSS, threads, CPU) from the kind node.
 *
 * Skipped unless SOAK_USERS is set, e.g.
 *   SOAK_USERS=25 SOAK_MINUTES=10 PROVIDER=keycloak CLUSTER=... FATHOM_HOSTED_IP=... \
 *     pnpm -C ui e2e:hosted soak
 * Writes a JSON summary to test-results/hosted-soak.json.
 */
import { execFileSync } from 'node:child_process'
import { mkdirSync, writeFileSync } from 'node:fs'
import { expect, test, type BrowserContext, type Page } from '@playwright/test'

import { CLUSTER, kubeUser, signIn } from './helpers'
import { ensureUser } from './provider'

const USERS = Number(process.env.SOAK_USERS ?? 0)
const MINUTES = Number(process.env.SOAK_MINUTES ?? 10)
const name = (i: number): string => `soak-${String(i).padStart(2, '0')}`

interface Sample {
  t: number
  rssMiB: number
  threads: number
  cpuSeconds: number
}

/** fathom-server's /proc numbers, read from the kind node (the process is visible there). */
function sample(t0: number): Sample {
  const out = execFileSync(
    'docker',
    [
      'exec',
      `${CLUSTER}-control-plane`,
      'sh',
      '-c',
      'p=$(pgrep -x fathom-server | head -1); grep -E "^(VmRSS|Threads):" /proc/$p/status; cut -d" " -f14,15 /proc/$p/stat',
    ],
    { encoding: 'utf8' },
  )
  const rss = Number(/VmRSS:\s+(\d+)/.exec(out)?.[1] ?? 0) / 1024
  const threads = Number(/Threads:\s+(\d+)/.exec(out)?.[1] ?? 0)
  const [ut = 0, st = 0] = (out.trim().split('\n').at(-1) ?? '').split(' ').map(Number)
  return {
    t: (Date.now() - t0) / 1000,
    rssMiB: Math.round(rss),
    threads,
    cpuSeconds: (ut + st) / 100,
  }
}

interface UserStats {
  identity: string | null
  sent: number
  finished: number
  failed: number
  errors: number
  closed: number | null
  batches: number
  envelopes: number
  dropped: number
  foreignMarkers: string[]
  foreignRequests: number
  ownMarkerSeen: boolean
  latencies: number[]
}

/** In-page harness: one raw /ws, a command loop, and cross-user checks on every frame. */
async function startLoad(page: Page, index: number, minutes: number): Promise<void> {
  await page.evaluate(
    ({ index, minutes }) => {
      const own = `soak-${index}-marker`
      const st = {
        identity: null as string | null,
        sent: 0,
        finished: 0,
        failed: 0,
        errors: 0,
        closed: null as number | null,
        batches: 0,
        envelopes: 0,
        dropped: 0,
        foreignMarkers: [] as string[],
        foreignRequests: 0,
        ownMarkerSeen: false,
        latencies: [] as number[],
      }
      const pendingSends: number[] = []
      const inflight = new Map<number, number>()
      ;(window as unknown as { __soak: typeof st }).__soak = st
      const ws = new WebSocket(`wss://${location.host}/ws`)
      ws.onclose = (e) => (st.closed = e.code)
      ws.onmessage = (e: MessageEvent<string>) => {
        for (const m of e.data.matchAll(/soak-(\d+)-marker/g)) {
          if (Number(m[1]) === index) st.ownMarkerSeen = true
          else if (!st.foreignMarkers.includes(m[0])) st.foreignMarkers.push(m[0])
        }
        const f = JSON.parse(e.data) as {
          op: string
          request_id?: number
          info?: { identity?: { username?: string } }
          batch?: {
            dropped: number
            envelopes: {
              correlation: { type: string; data?: number }
              event: { type: string; data: { ok?: boolean } }
            }[]
          }
        }
        if (f.op === 'subscribed') st.identity = f.info?.identity?.username ?? null
        else if (f.op === 'error') st.errors++
        else if (f.op === 'sent') {
          const t = pendingSends.shift()
          if (t !== undefined && f.request_id !== undefined) inflight.set(f.request_id, t)
        } else if (f.op === 'batch') {
          st.batches++
          st.dropped += f.batch?.dropped ?? 0
          for (const env of f.batch?.envelopes ?? []) {
            st.envelopes++
            if (env.event.type !== 'CommandFinished' || env.correlation.type !== 'Request') continue
            const rid = env.correlation.data ?? -1
            const t = inflight.get(rid)
            if (t === undefined) {
              st.foreignRequests++
              continue
            }
            inflight.delete(rid)
            st.finished++
            if (!env.event.data.ok) st.failed++
            st.latencies.push(performance.now() - t)
          }
        }
      }
      const cmds = [
        { type: 'RefreshNamespaces' },
        { type: 'SelectNamespace', data: { name: 'demo' } },
        { type: 'RefreshPods' },
        { type: 'RefreshDeployments' },
        { type: 'SelectDeployment', data: { namespace: 'demo', name: 'rbac-probe' } },
        { type: 'SelectDeployment', data: { namespace: 'demo', name: own } },
        { type: 'RefreshServices' },
        { type: 'RefreshNodes' },
      ]
      const end = performance.now() + minutes * 60_000
      let n = 0
      const send = (cmd: unknown) => {
        pendingSends.push(performance.now())
        st.sent++
        ws.send(JSON.stringify({ op: 'send', cmd }))
      }
      ws.onopen = () => {
        ws.send(JSON.stringify({ op: 'subscribe' }))
        const tick = () => {
          if (performance.now() > end || st.closed !== null) return
          send(cmds[n++ % cmds.length])
          setTimeout(tick, 2_000 + Math.random() * 3_000)
        }
        setTimeout(tick, Math.random() * 2_000)
      }
    },
    { index, minutes },
  )
}

const stats = (page: Page): Promise<UserStats> =>
  page.evaluate(() => (window as unknown as { __soak: UserStats }).__soak)

test('soak: N users browse and watch in isolation', async ({ browser }) => {
  test.skip(!USERS, 'set SOAK_USERS to run the soak')
  test.setTimeout((MINUTES + 20) * 60_000)
  const t0 = Date.now()
  const samples: Sample[] = [sample(t0)]
  for (let i = 1; i <= USERS; i++) await ensureUser(name(i), 'fathom-viewers')

  const users: { ctx: BrowserContext; page: Page }[] = []
  for (let i = 1; i <= USERS; i++) {
    const ctx = await browser.newContext({
      baseURL: 'https://fathom.localtest.me',
      ignoreHTTPSErrors: true,
    })
    const page = await ctx.newPage()
    await signIn(page, name(i))
    await page.goto('/healthz') // light page on the app origin: the raw socket is the only client
    users.push({ ctx, page })
  }
  samples.push(sample(t0))
  for (const [k, u] of users.entries()) await startLoad(u.page, k + 1, MINUTES)

  const deadline = Date.now() + MINUTES * 60_000
  while (Date.now() < deadline) {
    await new Promise((r) => setTimeout(r, 30_000))
    samples.push(sample(t0))
  }
  await new Promise((r) => setTimeout(r, 15_000)) // let the last commands finish
  const all = await Promise.all(users.map((u) => stats(u.page)))
  samples.push(sample(t0))
  for (const u of users) await u.ctx.close()

  const lat = all.flatMap((s) => s.latencies).sort((a, b) => a - b)
  const pct = (p: number) =>
    Math.round(lat[Math.min(lat.length - 1, Math.floor(lat.length * p))] ?? 0)
  const cpu = samples.slice(2).map((s, k) => {
    const prev = samples[k + 1] ?? s
    return s.t > prev.t ? ((s.cpuSeconds - prev.cpuSeconds) / (s.t - prev.t)) * 100 : 0
  })
  const [idle, signedIn] = [
    samples[0] ?? samples[samples.length - 1],
    samples[1] ?? samples[0],
  ] as [Sample, Sample]
  const summary = {
    users: USERS,
    minutes: MINUTES,
    provider: process.env.PROVIDER ?? 'rauthy',
    commands: {
      sent: all.reduce((a, s) => a + s.sent, 0),
      finished: all.reduce((a, s) => a + s.finished, 0),
    },
    latencyMs: { p50: pct(0.5), p95: pct(0.95), max: Math.round(lat.at(-1) ?? 0) },
    envelopes: all.reduce((a, s) => a + s.envelopes, 0),
    dropped: all.reduce((a, s) => a + s.dropped, 0),
    rssMiB: {
      idle: idle.rssMiB,
      signedIn: signedIn.rssMiB,
      peak: Math.max(...samples.map((s) => s.rssMiB)),
    },
    threads: { idle: idle.threads, peak: Math.max(...samples.map((s) => s.threads)) },
    cpuPercent: {
      mean: Math.round(cpu.reduce((a, b) => a + b, 0) / Math.max(1, cpu.length)),
      max: Math.round(Math.max(0, ...cpu)),
    },
    perUser: all.map((s, k) => ({ user: name(k + 1), ...s, latencies: s.latencies.length })),
    samples,
  }
  mkdirSync('test-results', { recursive: true })
  writeFileSync('test-results/hosted-soak.json', JSON.stringify(summary, null, 2))
  console.log(JSON.stringify({ ...summary, perUser: undefined, samples: undefined }, null, 2))

  for (const [k, s] of all.entries()) {
    const who = name(k + 1)
    expect(s.identity, who).toBe(kubeUser(who))
    expect(s.foreignMarkers, who).toEqual([])
    expect(s.foreignRequests, who).toBe(0)
    expect(s.ownMarkerSeen, who).toBe(true)
    expect(s.errors, who).toBe(0)
    expect(s.closed, who).toBeNull()
    expect(s.finished, who).toBe(s.sent)
  }
})
