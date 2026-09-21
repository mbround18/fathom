/** Live e2e global teardown: stop the `fathom-devbridge` the global setup started. */
import { ENV } from './env'

function alive(pid: number): boolean {
  try {
    process.kill(pid, 0)
    return true
  } catch {
    return false
  }
}

export default async function globalTeardown(): Promise<void> {
  const pid = Number(process.env[ENV.pid])
  if (!pid || !alive(pid)) return
  process.kill(pid, 'SIGTERM')
  const deadline = Date.now() + 5_000
  while (alive(pid) && Date.now() < deadline) await new Promise((r) => setTimeout(r, 100))
  if (alive(pid)) process.kill(pid, 'SIGKILL')
  console.log('[live e2e] fathom-devbridge stopped')
}
