// T120 / SC-005: with Pods open and idle for 60 s, the app process tree stays under 2% of one core.
import assert from 'node:assert/strict'
import { test } from 'node:test'

import { PODS_ROWS_JS, prerequisitesMissing, sleep, startSession, treeTicks } from './harness.ts'

const skip = prerequisitesMissing() ?? false
const HZ = 100 // CLK_TCK on Linux
const WINDOW_S = Number(process.env.IDLE_WINDOW_S ?? 60)

void test('idle CPU on the Pods view is under 2% of one core', { skip, timeout: 240_000 }, async () => {
  const s = await startSession()
  try {
    await s.waitFor<number>(PODS_ROWS_JS, (n) => n > 0, 90_000)
    await sleep(10_000) // settle
    const pid = s.pid()
    const t0 = treeTicks(pid)
    const w0 = Date.now()
    await sleep(WINDOW_S * 1000)
    const pct = ((treeTicks(pid) - t0) / HZ / ((Date.now() - w0) / 1000)) * 100
    console.log(`idle_cpu_percent_of_one_core=${pct.toFixed(2)}`)
    assert.ok(pct < 2, `idle CPU ${pct.toFixed(2)}% >= 2%`)
  } finally {
    await s.close()
  }
})
