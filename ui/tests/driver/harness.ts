// Minimal tauri-driver harness (no npm dependencies): speaks the W3C WebDriver protocol over fetch.
//
// Prerequisites (Linux only; tauri-driver has no macOS support):
//   cargo install tauri-driver --locked
//   WebKitWebDriver on PATH (Debian/Ubuntu: `sudo apt install webkit2gtk-driver`; its version must
//   match libwebkit2gtk-4.1). Set WEBKIT_DRIVER to its path if it is not on PATH.
//   A release binary: `pnpm -C ui build && cargo build --release -p fathom-desktop`
//   (override with FATHOM_BINARY=/path/to/fathom-desktop), and a display (X11/Wayland, or xvfb-run).
//   A reachable kubeconfig context for the Pods-view assertions.
// Run: `pnpm -C ui test:driver`.
import { type ChildProcess, spawn } from 'node:child_process'
import * as fs from 'node:fs'
import { resolve } from 'node:path'

const PORT = Number(process.env.TAURI_DRIVER_PORT ?? 4444)
const BASE = `http://127.0.0.1:${PORT}`
export const BINARY = resolve(
  process.env.FATHOM_BINARY ??
    new URL('../../../target/release/fathom-desktop', import.meta.url).pathname,
)

export const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms))

function onPath(name: string): boolean {
  if (name.includes('/')) return fs.existsSync(name)
  return (process.env.PATH ?? '').split(':').some((d) => fs.existsSync(`${d}/${name}`))
}

export function prerequisitesMissing(): string | null {
  if (!fs.existsSync(BINARY)) return `binary not found: ${BINARY}`
  if (!onPath(process.env.TAURI_DRIVER ?? 'tauri-driver')) return 'tauri-driver not found'
  if (!onPath(process.env.WEBKIT_DRIVER ?? 'WebKitWebDriver')) return 'WebKitWebDriver not found'
  return null
}

async function call(method: string, path: string, body?: unknown) {
  const res = await fetch(`${BASE}${path}`, {
    method,
    headers: { 'content-type': 'application/json' },
    body: body === undefined ? undefined : JSON.stringify(body),
  })
  const json = (await res.json()) as { value: unknown }
  if (!res.ok) throw new Error(`${method} ${path}: ${JSON.stringify(json.value)}`)
  return json.value
}

export interface Session {
  /** pid of the app process (found by exe path; tauri-driver launches it indirectly). */
  pid(): number
  waitFor<T>(script: string, ok: (v: T) => boolean, timeoutMs: number): Promise<T>
  close(): Promise<void>
}

export async function startSession(): Promise<Session> {
  const args = process.env.WEBKIT_DRIVER ? ['--native-driver', process.env.WEBKIT_DRIVER] : []
  const driver: ChildProcess = spawn(
    process.env.TAURI_DRIVER ?? 'tauri-driver',
    ['--port', String(PORT), ...args],
    { stdio: 'ignore' },
  )
  for (let i = 0; i < 50; i++) {
    try {
      await fetch(`${BASE}/status`)
      break
    } catch {
      await sleep(100)
    }
  }
  const created = (await call('POST', '/session', {
    capabilities: { alwaysMatch: { 'tauri:options': { application: BINARY } } },
  })) as { sessionId: string }
  const sid = created.sessionId
  const exec = async <T>(script: string) =>
    (await call('POST', `/session/${sid}/execute/sync`, { script, args: [] })) as T
  return {
    pid: findAppPid,
    async waitFor<T>(script: string, ok: (v: T) => boolean, timeoutMs: number) {
      const end = Date.now() + timeoutMs
      let last: T = undefined as T
      while (Date.now() < end) {
        try {
          last = await exec<T>(script)
          if (ok(last)) return last
        } catch {
          // The page can still be loading/unloading right after launch; retry.
        }
        await sleep(500)
      }
      throw new Error(`timed out; last value ${JSON.stringify(last)}`)
    },
    async close() {
      await call('DELETE', `/session/${sid}`).catch(() => undefined)
      driver.kill()
    },
  }
}

function findAppPid(): number {
  for (const d of fs.readdirSync('/proc')) {
    if (!/^\d+$/.test(d)) continue
    try {
      if (fs.readFileSync(`/proc/${d}/cmdline`, 'utf8').split('\0')[0] === BINARY) return Number(d)
    } catch {
      /* process gone */
    }
  }
  throw new Error('app process not found')
}

const statFields = (pid: number) => {
  const st = fs.readFileSync(`/proc/${pid}/stat`, 'utf8')
  return st.slice(st.lastIndexOf(')') + 2).split(' ') // [0]=state [1]=ppid ... [11]=utime [12]=stime
}

/** utime+stime ticks summed over a pid and all its descendants. */
export function treeTicks(root: number): number {
  const kids = new Map<number, number[]>()
  for (const d of fs.readdirSync('/proc')) {
    if (!/^\d+$/.test(d)) continue
    try {
      const ppid = Number(statFields(Number(d))[1])
      kids.set(ppid, [...(kids.get(ppid) ?? []), Number(d)])
    } catch {
      /* gone */
    }
  }
  let total = 0
  const walk = (p: number) => {
    try {
      const f = statFields(p)
      total += Number(f[11]) + Number(f[12])
    } catch {
      /* gone */
    }
    for (const c of kids.get(p) ?? []) walk(c)
  }
  walk(root)
  return total
}

/** Body rows in the resource grid (the header is also role=row). */
export const PODS_ROWS_JS = `return document.querySelectorAll('[role="row"]').length - 1`
