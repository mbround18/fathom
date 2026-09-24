import { defineConfig, devices } from '@playwright/test'

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

const ENGINES = [
  { engine: 'chromium', device: devices['Desktop Chrome'] },
  { engine: 'webkit', device: devices['Desktop Safari'] },
] as const

export default defineConfig({
  testDir: './tests/e2e',
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 2 : 0,
  reporter: process.env.CI ? [['list'], ['html', { open: 'never' }]] : 'list',
  use: {
    baseURL: 'http://localhost:4173',
    trace: 'retain-on-failure',
  },
  projects: ENGINES.flatMap(({ engine, device }) =>
    VIEWPORTS.map(({ name, viewport, touch, expect }) => ({
      name: `${engine}-${name}`,
      metadata: expect,
      use: {
        ...device,
        viewport,
        deviceScaleFactor: 1,
        hasTouch: touch ?? false,
      },
    })),
  ),
  webServer: {
    // A built fake-mode bundle, not the dev server: unbundled Vite modules under
    // fully parallel workers made page boot time out intermittently.
    command: 'pnpm e2e:serve',
    url: 'http://localhost:4173',
    reuseExistingServer: false,
    timeout: 60_000,
  },
})
