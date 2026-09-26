// T118: launch the real Tauri app under tauri-driver and confirm the Pods view populates from the
// real backend. Skips (with the reason) when the prerequisites in harness.ts are missing.
import assert from 'node:assert/strict'
import { test } from 'node:test'

import { PODS_ROWS_JS, prerequisitesMissing, startSession } from './harness.ts'

const skip = prerequisitesMissing() ?? false

void test('app launches and the Pods view populates', { skip, timeout: 120_000 }, async () => {
  const s = await startSession()
  try {
    const rows = await s.waitFor<number>(PODS_ROWS_JS, (n) => n > 0, 90_000)
    assert.ok(rows > 0, `expected pod rows, got ${rows}`)
  } finally {
    await s.close()
  }
})
