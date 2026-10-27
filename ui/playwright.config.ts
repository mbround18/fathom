import { defineConfig, devices } from '@playwright/test'

import { autoWorkers } from './tests/support/workers.js'

/**
 * One project per width class × engine (contracts/layout.md § Verification). Each project's
 * metadata carries what the shell must render at that viewport.
 *
 * Note: the contract's "wide 2560×1440" viewport is the ultra class under the breakpoints
 * (wide = 1920–2559), so wide is exercised at 1920×1080 and 2560×1440 runs as ultra.
 */
export interface ShellExpectation {
  widthClass: 'phone' | 'tablet' | 'laptop' | 'desktop' | 'wide' | 'ultra'
  slots: number
  dock: boolean
}

const VIEWPORTS: {
  name: string
  viewport: { width: number; height: number }
  touch?: boolean
  expect: ShellExpectation
}[] = [
  {
    name: 'phone',
    viewport: { width: 390, height: 844 },
    touch: true,
    expect: { widthClass: 'phone', slots: 1, dock: false },
  },
  {
    name: 'tablet',
    viewport: { width: 820, height: 1180 },
    touch: true,
    expect: { widthClass: 'tablet', slots: 2, dock: false },
  },
  {
    name: 'laptop',
    viewport: { width: 1280, height: 800 },
    expect: { widthClass: 'laptop', slots: 3, dock: true },
  },
  {
    name: 'desktop',
    viewport: { width: 1600, height: 900 },
    expect: { widthClass: 'desktop', slots: 3, dock: true },
  },
  {
    name: 'wide',
    viewport: { width: 1920, height: 1080 },
    expect: { widthClass: 'wide', slots: 4, dock: true },
  },
  {
    name: 'ultra',
    viewport: { width: 2560, height: 1440 },
    expect: { widthClass: 'ultra', slots: 5, dock: true },
  },
  {
    name: 'ultra-plus',
    viewport: { width: 5120, height: 1440 },
    expect: { widthClass: 'ultra', slots: 6, dock: true },
  },
]

/**
 * Port of the fake-mode preview (default 4173). `FATHOM_E2E_UI_PORT` moves it, so several
 * worktrees can run the mock suite at the same time.
 */
const PORT = Number(process.env.FATHOM_E2E_UI_PORT ?? 4173)
const ORIGIN = `http://localhost:${String(PORT)}`

const ENGINES = [
  { engine: 'chromium', device: devices['Desktop Chrome'] },
  { engine: 'webkit', device: devices['Desktop Safari'] },
] as const

/** Wall-clock performance budgets: run only by the serial `*-perf` projects below. */
const PERF_SPECS = /perf-.*\.spec\.ts$/

/** The desktop-class viewport SC-004's budget is defined for (the `wide` class). */
const WIDE = {
  viewport: { width: 1920, height: 1080 },
  expect: { widthClass: 'wide', slots: 4, dock: true },
} satisfies { viewport: { width: number; height: number }; expect: ShellExpectation }

const FUNCTIONAL_PROJECTS = ENGINES.flatMap(({ engine, device }) =>
  VIEWPORTS.map(({ name, viewport, touch, expect }) => ({
    name: `${engine}-${name}`,
    testIgnore: PERF_SPECS,
    // WebKit's WPE compositor thread sometimes segfaults (WebKit bug 308242) and takes the page
    // with it. One retry absorbs that, and tests/reporters/engine-crash-gate.ts fails the run if a
    // retry hid anything else.
    ...(engine === 'webkit' && !process.env.CI ? { retries: 1 } : {}),
    metadata: expect,
    use: {
      ...device,
      viewport,
      deviceScaleFactor: 1,
      hasTouch: touch ?? false,
    },
  })),
)

const workers = autoWorkers()

export default defineConfig({
  testDir: './tests/e2e',
  fullyParallel: true,
  // Sized from the machine's current load and free memory rather than Playwright's default of
  // half the cores, which on a 32-core box is 16 browsers and enough to make the desktop crawl
  // while another agent is building. `PLAYWRIGHT_WORKERS=<n>` overrides.
  ...(workers === undefined ? {} : { workers }),
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 2 : 0,
  reporter: [
    ['list'],
    ...(process.env.CI ? [['html', { open: 'never' }] as const] : []),
    // Fails the run if a retry masked anything but a browser-engine crash.
    ['./tests/reporters/engine-crash-gate.ts'],
  ],
  use: {
    baseURL: ORIGIN,
    trace: 'retain-on-failure',
  },
  projects: [
    ...FUNCTIONAL_PROJECTS,
    // SC-004's keystroke budget is wall-clock, so it is only meaningful without CPU contention:
    // under the functional projects' parallel workers, a 10k-row filter that takes ~30 ms idle
    // was measured at 110-470 ms. These projects depend on every functional project, so they run
    // after all of them, one test at a time, on the `wide` desktop-class viewport the budget is
    // defined for. Run just the perf check with
    // `pnpm e2e --project chromium-perf --project webkit-perf --no-deps`.
    ...ENGINES.map(({ engine, device }) => ({
      name: `${engine}-perf`,
      testMatch: PERF_SPECS,
      dependencies: FUNCTIONAL_PROJECTS.map((p) => p.name),
      workers: 1,
      metadata: WIDE.expect,
      use: { ...device, viewport: WIDE.viewport, deviceScaleFactor: 1, hasTouch: false },
    })),
  ],
  webServer: {
    // A built fake-mode bundle, not the dev server: unbundled Vite modules under
    // fully parallel workers made page boot time out intermittently.
    command:
      'pnpm exec vite build --mode fake --outDir dist-e2e && ' +
      `pnpm exec vite preview --outDir dist-e2e --port ${String(PORT)} --strictPort`,
    url: ORIGIN,
    reuseExistingServer: false,
    timeout: 60_000,
  },
})
