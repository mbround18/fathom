/**
 * SC-004 (T046): 200 stacks and 2,000 containers render in under 2 s and filter in under 100 ms.
 */
import { expect, fixtures as f, test } from '../../support/fake-backend'

const STACKS = 200
const PER_STACK = 10

function bigList() {
  const stacks = Array.from({ length: STACKS }, (_, i) => {
    const name = `stack-${String(i).padStart(3, '0')}`
    const containers = Array.from({ length: PER_STACK }, (_, j) =>
      f.container({ name: `${name}-app-${String(j + 1)}`, project: name, service: 'app' }),
    )
    return f.stack(name, {
      status: i % 7 === 0 ? 'stopped' : 'running',
      services: [{ name: 'app', image: 'nginx:1.29', containers }],
    })
  })
  return f.stackList(stacks)
}

test('200 stacks / 2,000 containers: first render under 2 s, filtering under 100 ms', async ({ page, backend }) => {
  backend.stacks = bigList()
  const start = Date.now()
  await page.goto('/')
  await expect(page.locator('[data-stack="stack-000"]')).toBeVisible()
  const firstRender = Date.now() - start
  expect(firstRender, `first render took ${String(firstRender)} ms`).toBeLessThan(2000)
  await expect(page.getByTestId('stack-count')).toHaveText('200 of 200')
  // Virtualised: only a screenful of rows is in the DOM.
  expect(await page.locator('[data-stack]').count()).toBeLessThan(80)

  // `stack-19` matches stack-190 to stack-199. Time from the input event to the next painted frame showing the filtered list.
  const filterMs = await page.evaluate(async () => {
    const input = document.querySelector<HTMLInputElement>('input[aria-label="Filter by name"]')
    if (!input) throw new Error('no filter input')
    const t0 = performance.now()
    // The prototype setter, with the input as receiver, so React's value tracker sees a change.
    Reflect.set(HTMLInputElement.prototype, 'value', 'stack-19', input)
    input.dispatchEvent(new Event('input', { bubbles: true }))
    for (;;) {
      await new Promise((r) => requestAnimationFrame(r))
      if (document.querySelector('[data-testid="stack-count"]')?.textContent === '10 of 200') break
      if (performance.now() - t0 > 2000) break
    }
    return performance.now() - t0
  })
  expect(filterMs, `filtering took ${filterMs.toFixed(1)} ms`).toBeLessThan(100)
  await expect(page.locator('[data-stack]')).toHaveCount(10)
})
