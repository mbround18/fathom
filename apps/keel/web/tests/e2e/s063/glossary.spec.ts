/**
 * Spec 063 T036 (US3, FR-020 to FR-022, FR-024): one glossary term per page opens its info tip on
 * hover, on keyboard focus and on a tap; Escape and a tap elsewhere close it; at 360 px it fits the
 * screen; and a term reads the same on every page. Against the fake backend.
 */
import type { Locator, Page } from '@playwright/test'

import { expect, fixtures, test, type FakeBackend } from '../../support/fake-backend'
import { governance, people } from '../../support/fake-governance'
import { FakeHosts } from '../../support/fake-hosts'
import { WORKSPACE } from '../../support/fake-repos'
import { FakeReviews } from '../../support/fake-reviews'

/** A glossary id (`src/help/glossary.ts`). Not imported: that module's guide types need Vite. */
type TermId = 'trusted-signers' | 'fingerprint' | 'credential' | 'receiver' | 'inventory-managed' | 'grant' | 'workspace'

const verified = fixtures.verification({ verified: true, enrolled: true, at: '2026-10-27T11:59:00Z' })

/** A workspace admin of `ops`, with the workspace listed and hosts answered. */
function admin(backend: FakeBackend): void {
  backend.me = fixtures.me({
    grants: [{ workspace: WORKSPACE, workspace_slug: 'ops', role: 'ws_admin', profiles: [] }],
    verification: verified,
  })
  backend.on('ListWorkspaces', () => [
    {
      type: 'Workspaces',
      data: {
        items: [
          { id: WORKSPACE, slug: 'ops', name: 'Ops', created: '2026-10-27T12:00:00Z', archived: false, projects: [], system: false },
        ],
      },
    },
  ])
  new FakeHosts(backend)
}

interface PageCase {
  name: string
  term: TermId
  /** The icon's accessible name. */
  about: string
  /** The start of the glossary text. */
  text: string
  /** Sets up the backend, opens the page and shows the section the term sits in. */
  open: (page: Page, backend: FakeBackend) => Promise<void>
}

const PAGES: readonly PageCase[] = [
  {
    name: 'Reviews',
    term: 'trusted-signers',
    about: 'About trusted signers',
    text: 'SSH or OpenPGP keys whose signed commits',
    open: async (page, backend) => {
      admin(backend)
      new FakeReviews(backend)
      await page.goto('/reviews')
    },
  },
  {
    name: 'Hosts',
    term: 'fingerprint',
    about: 'About fingerprint',
    text: 'A short code that identifies a key',
    open: async (page, backend) => {
      admin(backend)
      await page.goto('/hosts')
      await page.getByText('SSH certificate authority').click()
    },
  },
  {
    name: 'Bindings',
    term: 'credential',
    about: 'About credential',
    text: 'A secret a binding uses to fetch the repository',
    open: async (page, backend) => {
      admin(backend)
      await page.goto('/bindings')
    },
  },
  {
    name: 'Receivers',
    term: 'receiver',
    about: 'About receiver',
    text: 'A webhook address with its own secret',
    open: async (page, backend) => {
      admin(backend)
      await page.goto('/receivers')
    },
  },
  {
    name: 'Inventory',
    term: 'inventory-managed',
    about: 'About managed inventory',
    text: 'An inventory you write and edit in Keel',
    open: async (page, backend) => {
      admin(backend)
      await page.goto('/inventory')
      await page.locator('header').getByRole('button', { name: 'New inventory' }).click()
    },
  },
  {
    name: 'Teams',
    term: 'grant',
    about: 'About grant',
    text: 'Gives a team a role in one workspace',
    open: async (page, backend) => {
      backend.me = fixtures.me({ instance_role: 'owner', verification: verified })
      governance(backend, (g) => {
        const ws = g.workspace('ops', 'Ops')
        g.team('Platform', [people.ada()], [{ workspace: ws.id, workspace_slug: 'ops', role: 'viewer', profiles: [] }])
      })
      await page.goto('/teams')
      await page.locator('[data-team="Platform"]').getByRole('button', { name: /Platform/ }).click()
    },
  },
]

const trigger = (page: Page, term: TermId): Locator => page.locator(`[data-infotip="${term}"]`).first()
const content = (page: Page, term: TermId): Locator => page.locator(`[data-infotip-content="${term}"]`)

/** Presses Tab until `target` has focus, as a keyboard user would reach it. */
async function tabTo(page: Page, target: Locator): Promise<void> {
  await page.locator('body').focus()
  for (let i = 0; i < 80; i++) {
    await page.keyboard.press('Tab')
    if (await target.evaluate((el) => el === document.activeElement)) return
  }
  throw new Error('Tab never reached the info tip')
}

for (const p of PAGES) {
  test(`${p.name}: the ${p.term} tip opens on hover and focus, and Escape closes it`, async ({ page, backend }) => {
    await p.open(page, backend)
    const icon = trigger(page, p.term)
    const tip = content(page, p.term)
    await expect(icon).toHaveAccessibleName(p.about)

    // Hover opens it; moving away closes it.
    await icon.hover()
    await expect(tip).toBeVisible()
    await expect(tip).toContainText(p.text)
    await page.mouse.move(0, 0)
    await expect(tip).toBeHidden()

    // Tab focus opens it, and the icon is described by it.
    await tabTo(page, icon)
    await expect(tip).toBeVisible()
    const id = await tip.getAttribute('id')
    expect(id).not.toBeNull()
    await expect(icon).toHaveAttribute('aria-describedby', id ?? '')

    await page.keyboard.press('Escape')
    await expect(tip).toBeHidden()
    await expect(icon).not.toHaveAttribute('aria-describedby', /.+/)
  })
}

test.describe('on a phone', () => {
  test.use({ hasTouch: true, viewport: { width: 360, height: 740 } })

  for (const p of PAGES) {
    test(`${p.name}: a tap opens the ${p.term} tip inside the screen, a tap elsewhere closes it`, async ({ page, backend }) => {
      await p.open(page, backend)
      const icon = trigger(page, p.term)
      const tip = content(page, p.term)

      await icon.scrollIntoViewIfNeeded()
      await icon.tap()
      await expect(tip).toBeVisible()
      const box = await tip.boundingBox()
      expect(box).not.toBeNull()
      if (box) {
        expect(box.x).toBeGreaterThanOrEqual(0)
        expect(box.x + box.width).toBeLessThanOrEqual(360)
      }
      expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(360)

      // A tap on the page's title, away from the tip.
      const title = page.getByRole('heading', { level: 1 })
      await title.scrollIntoViewIfNeeded()
      const at = await title.boundingBox()
      expect(at).not.toBeNull()
      if (at) await page.touchscreen.tap(at.x + 4, at.y + at.height / 2)
      await expect(tip).toBeHidden()
    })
  }
})

test('a term reads the same on every page', async ({ page, backend }) => {
  admin(backend)
  new FakeReviews(backend)
  const texts: string[] = []
  for (const path of ['/hosts', '/reviews']) {
    await page.goto(path)
    const icon = trigger(page, 'workspace')
    await icon.focus()
    const tip = content(page, 'workspace')
    await expect(tip).toBeVisible()
    texts.push((await tip.textContent()) ?? '')
    await page.keyboard.press('Escape')
  }
  expect(texts[0]).toContain('Keeps a set of hosts, bindings, inventories and runs apart')
  expect(texts[1]).toBe(texts[0])
})
