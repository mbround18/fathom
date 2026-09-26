/**
 * Shells and port forwards run with the signed-in user's identity (spec 031 US1, SC-002):
 * the API server allows `pods/exec` and `pods/portforward` for bob (edit in `demo`) and refuses
 * them for alice (view).
 *
 * The port-forward listener binds inside the fathom-server pod, so the specs connect to it from
 * an ephemeral debug container sharing that pod's network namespace (see helpers.ts).
 */
import { expect, test } from '@playwright/test'

import { b64, connectFromServerPod, probePod, RawWs, signIn, unb64 } from './helpers'

interface Chunk {
  stream_id: number
  data: string
}

test('bob gets a shell in demo; alice is refused', async ({ browser }) => {
  const pod = probePod()
  const bobCtx = await browser.newContext({
    baseURL: 'https://fathom.localtest.me',
    ignoreHTTPSErrors: true,
  })
  const bob = await bobCtx.newPage()
  await signIn(bob, 'bob')
  const raw = await RawWs.open(bob)
  const start = await raw.command({
    type: 'StartPodExec',
    data: { namespace: 'demo', name: pod, container: null },
  })
  expect(start.ok, start.error ?? '').toBe(true)
  await expect
    .poll(async () => (await raw.events('Stream', 'PodExecReady')).length)
    .toBeGreaterThan(0)
  const marker = `fathom-exec-${Date.now()}`
  await raw.command({ type: 'SendPodExecInput', data: { data: b64(`echo ${marker}\n`) } })
  await expect
    .poll(async () =>
      (await raw.events<Chunk>('Stream', 'PodExecOutputChunk')).map((c) => unb64(c.data)).join(''),
    )
    .toContain(marker)
  await raw.command({ type: 'StopPodExec' })
  await bobCtx.close()

  const aliceCtx = await browser.newContext({
    baseURL: 'https://fathom.localtest.me',
    ignoreHTTPSErrors: true,
  })
  const alice = await aliceCtx.newPage()
  await signIn(alice, 'alice')
  const rawA = await RawWs.open(alice)
  const check = await rawA.command({
    type: 'CheckPodExecAccess',
    data: { namespace: 'demo', name: pod, container: null },
  })
  expect(check.ok).toBe(true)
  const verdicts = await rawA.events<{ allowed: boolean }>('ResourceRead', 'PodExecAccessChecked')
  expect(verdicts.at(-1)?.allowed).toBe(false)
  const denied = await rawA.command({
    type: 'StartPodExec',
    data: { namespace: 'demo', name: pod, container: null },
  })
  const closed = await rawA.events<{ reason: string }>('Stream', 'StreamClosed')
  const text = `${denied.error ?? ''} ${closed.map((c) => c.reason).join(' ')}`
  expect(denied.ok && closed.length === 0).toBe(false)
  expect(text).toMatch(/forbidden/i)
  expect((await rawA.events('Stream', 'PodExecReady')).length).toBe(0)
  await aliceCtx.close()
})

test('port-forward: bob reaches the pod, alice is refused by the API server', async ({
  browser,
}) => {
  test.setTimeout(180_000)
  const pod = probePod()
  const run = async (user: string, port: number): Promise<string> => {
    const ctx = await browser.newContext({
      baseURL: 'https://fathom.localtest.me',
      ignoreHTTPSErrors: true,
    })
    const page = await ctx.newPage()
    await signIn(page, user)
    const raw = await RawWs.open(page)
    const start = await raw.command({
      type: 'StartPortForward',
      data: { namespace: 'demo', pod, local_port: port, remote_port: 8080, container: null },
    })
    expect(start.ok, start.error ?? '').toBe(true)
    const out = connectFromServerPod(port)
    await raw.command({ type: 'RefreshPortForwards' })
    await ctx.close()
    return out
  }
  const bobOut = await run('bob', 18080 + Math.floor(Math.random() * 500))
  expect(bobOut).toContain('fathom-pf-ok')
  const aliceOut = await run('alice', 18600 + Math.floor(Math.random() * 500))
  expect(aliceOut).not.toContain('fathom-pf-ok')
  expect(aliceOut).toContain('Port forward error')
})
