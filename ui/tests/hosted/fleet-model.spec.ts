/**
 * 046-fleet-model in hosted mode (FR-018): the fleet *model* is allowed, the fleet *poller* is not.
 *
 * Hosted Fathom serves exactly one cluster, reached as the context `home`. That makes the 046
 * distinction load-bearing rather than pedantic: naming, labelling and grouping are operator
 * metadata in the state directory and touch no cluster at all, so they work here; polling and
 * federation read the kubeconfig and talk to other clusters, so they stay denied. A build that
 * blanket-denied everything with "Fleet" in its name would take the operator's ability to label
 * their one cluster away for no reason, and a build that allowed everything would let a hosted
 * client enumerate clusters it was never given.
 *
 * Live suite — needs the hosted cluster up (see playwright.hosted.config.ts):
 *   PROVIDER=rauthy k8s/cluster.sh up-hosted
 *   PROVIDER=rauthy pnpm -C ui e2e:hosted fleet-model
 */
import { type Browser, expect, test } from '@playwright/test'

import { APP, RawWs, signIn } from './helpers'

/** The only context a hosted server ever has. */
const HOME = 'home'

const open = async (browser: Browser) => {
  const ctx = await browser.newContext({ baseURL: APP, ignoreHTTPSErrors: true })
  const page = await ctx.newPage()
  // bob holds `edit` in `demo`, which is what the Editor-gated publish needs.
  await signIn(page, 'bob')
  return { ctx, raw: await RawWs.open(page) }
}

interface Member {
  context: string
  display_name: string | null
  labels: Record<string, string>
}
interface Model {
  members: Member[]
}

test('naming, labelling and sharing the home cluster all work hosted', async ({ browser }) => {
  const { ctx, raw } = await open(browser)

  const named = await raw.command({
    type: 'SetFleetMemberDisplayName',
    data: { context: HOME, display_name: 'Hosted home' },
  })
  expect(named.ok, named.error ?? '').toBe(true)

  const labelled = await raw.command({
    type: 'SetFleetMemberLabels',
    data: { context: HOME, labels: { env: 'hosted', tier: 'prod' } },
  })
  expect(labelled.ok, labelled.error ?? '').toBe(true)

  // Assert on the emitted model rather than on `ok`: a command the server accepted and then
  // quietly dropped would still report ok, and the operator would be told their label stuck.
  await expect
    .poll(async () => {
      const models = await raw.events<Model>('System', 'FleetModelUpdated')
      const home = models.at(-1)?.members.find((m) => m.context === HOME)
      return { name: home?.display_name ?? null, env: home?.labels.env ?? null }
    })
    .toEqual({ name: 'Hosted home', env: 'hosted' })

  const grouped = await raw.command({
    type: 'SaveFleetGroup',
    data: { name: 'hosted', selector: 'env=hosted', previous_name: null },
  })
  expect(grouped.ok, grouped.error ?? '').toBe(true)

  const shared = await raw.command({ type: 'RefreshSharedFleetDefinitions' })
  expect(shared.ok, shared.error ?? '').toBe(true)

  await ctx.close()
})

test('a fleet command naming any other context is denied', async ({ browser }) => {
  const { ctx, raw } = await open(browser)

  // Every command that takes a context is a way to ask about a cluster, so each one is checked.
  // `home` is the whole fleet here; a server that accepted `kind-something-else` would be
  // answering for a cluster this client was never granted, and the denial is the only thing
  // standing between the two.
  for (const cmd of [
    { type: 'SetFleetMemberDisplayName', data: { context: 'kind-elsewhere', display_name: 'x' } },
    { type: 'SetFleetMemberLabels', data: { context: 'kind-elsewhere', labels: { a: 'b' } } },
    { type: 'ForgetFleetMember', data: { context: 'kind-elsewhere' } },
    { type: 'RequestFleetGroupRollup', data: { group: 'hosted', contexts: ['kind-elsewhere'] } },
  ]) {
    const denied = await raw.command(cmd)
    expect(denied.ok, `${cmd.type} naming another context was accepted`).toBe(false)
    expect(denied.error ?? '').toMatch(/context|hosted|denied|not permitted/i)
  }

  // And the model still describes only the one cluster: a denial that nonetheless created the
  // member would leak the name back on the next refresh.
  await raw.command({ type: 'RefreshFleetModel' })
  await expect
    .poll(async () => {
      const models = await raw.events<Model>('System', 'FleetModelUpdated')
      return (models.at(-1)?.members ?? []).map((m) => m.context)
    })
    .toEqual([HOME])

  await ctx.close()
})

test('the four pre-existing fleet denials still deny', async ({ browser }) => {
  const { ctx, raw } = await open(browser)

  // 046 added commands to this family; it must not have widened what the family may do. These
  // four read the kubeconfig or reach other clusters, which is exactly what hosted has none of.
  for (const cmd of [
    { type: 'StartFleetPolling' },
    { type: 'StopFleetPolling' },
    { type: 'RequestFederationSnapshot' },
    { type: 'RefreshContexts' },
  ]) {
    const denied = await raw.command(cmd)
    expect(denied.ok, `${cmd.type} is no longer denied in hosted mode`).toBe(false)
  }

  await ctx.close()
})
