/**
 * Live e2e global setup: prove the target is the labelled sandbox (read-only), then build and
 * start `fathom-devbridge` pinned to `$FATHOM_LIVE_CONTEXT` with writes confined to `fathom-dev`.
 *
 * The bridge's port and token reach the specs through `process.env` (workers inherit it), and
 * from there through the `app` fixture in helpers.ts. Nothing is written to the repo.
 */
import { execFileSync, spawn } from 'node:child_process'
import { randomBytes } from 'node:crypto'
import { createServer, connect } from 'node:net'
import { fileURLToPath } from 'node:url'

import { ENV, LIVE_NAMESPACE, SANDBOX_LABEL, liveContext } from './env'

const REPO_ROOT = fileURLToPath(new URL('../../..', import.meta.url))
/** Sandbox check, `SetContext`, and listening all happen before the port opens. */
const BRIDGE_START_TIMEOUT_MS = 180_000

function fail(message: string): never {
  throw new Error(`\n\n[live e2e] ${message}\n`)
}

/** Read-only: `kubectl get ns fathom-dev -o json` must show `fathom.dev/sandbox=true`. */
function checkSandboxLabel(context: string): void {
  const upHint =
    `Create it with:\n\n    FATHOM_SANDBOX_CONTEXT=${context} scripts/sandbox.sh up\n\n` +
    `(run from the repo root; see specs/001-tauri-shadcn-frontend/quickstart.md § Live e2e).`
  let raw: string
  try {
    raw = execFileSync(
      'kubectl',
      ['--context', context, 'get', 'namespace', LIVE_NAMESPACE, '-o', 'json'],
      { encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] },
    )
  } catch (err) {
    const stderr = (err as { stderr?: string }).stderr?.trim() ?? String(err)
    fail(
      `Could not read namespace '${LIVE_NAMESPACE}' in context '${context}':\n  ${stderr}\n\n${upHint}`,
    )
  }
  const ns = JSON.parse(raw) as { metadata?: { labels?: Record<string, string> } }
  const [key, value] = SANDBOX_LABEL
  if (ns.metadata?.labels?.[key] !== value) {
    fail(
      `Namespace '${LIVE_NAMESPACE}' in context '${context}' is not labelled ${key}=${value}; ` +
        `the live suite refuses to touch it.\n\n${upHint}`,
    )
  }
}

/** `cargo build -p fathom-devbridge`, returning the built executable's path. */
function buildDevbridge(): string {
  console.log('[live e2e] building fathom-devbridge…')
  let out: string
  try {
    out = execFileSync(
      'cargo',
      ['build', '-p', 'fathom-devbridge', '--message-format=json-render-diagnostics'],
      {
        cwd: REPO_ROOT,
        encoding: 'utf8',
        stdio: ['ignore', 'pipe', 'inherit'],
        maxBuffer: 256 * 1024 * 1024,
      },
    )
  } catch {
    fail('`cargo build -p fathom-devbridge` failed (see the compiler output above).')
  }
  for (const line of out.split('\n')) {
    if (!line.startsWith('{')) continue
    const msg = JSON.parse(line) as {
      reason?: string
      target?: { name?: string }
      executable?: string | null
    }
    if (msg.reason === 'compiler-artifact' && msg.target?.name === 'fathom-devbridge') {
      if (msg.executable) return msg.executable
    }
  }
  return fail('cargo did not report a fathom-devbridge executable.')
}

async function freePort(): Promise<number> {
  return new Promise((resolve, reject) => {
    const server = createServer()
    server.once('error', reject)
    server.listen(0, '127.0.0.1', () => {
      const addr = server.address()
      const port = typeof addr === 'object' && addr ? addr.port : 0
      server.close(() => {
        resolve(port)
      })
    })
  })
}

function portOpen(port: number): Promise<boolean> {
  return new Promise((resolve) => {
    const socket = connect({ host: '127.0.0.1', port })
    socket.once('connect', () => {
      socket.destroy()
      resolve(true)
    })
    socket.once('error', () => {
      socket.destroy()
      resolve(false)
    })
  })
}

export default async function globalSetup(): Promise<void> {
  const context = liveContext()
  checkSandboxLabel(context)
  const binary = buildDevbridge()

  const port = process.env[ENV.port] ? Number(process.env[ENV.port]) : await freePort()
  const token = randomBytes(24).toString('base64url')

  console.log(
    `[live e2e] starting fathom-devbridge on 127.0.0.1:${String(port)} ` +
      `(context '${context}', writes only in '${LIVE_NAMESPACE}')`,
  )
  const child = spawn(
    binary,
    ['--context', context, '--write-namespace', LIVE_NAMESPACE, '--port', String(port)],
    {
      cwd: REPO_ROOT,
      // The token goes through the environment, never argv (visible in `ps`).
      // The devbridge has no OS keyring; a debug-only in-memory secret store lets the Grafana
      // key specs run (release builds ignore this variable).
      env: { ...process.env, FATHOM_DEVBRIDGE_TOKEN: token, FATHOM_DEV_MEMORY_SECRETS: '1' },
      stdio: ['ignore', 'inherit', 'inherit'],
    },
  )
  let exited: string | undefined
  child.once('exit', (code, signal) => {
    exited = `exited with ${signal ?? `code ${String(code)}`}`
  })
  // Never leave a bridge behind, even if teardown does not run.
  process.once('exit', () => child.kill('SIGKILL'))

  const deadline = Date.now() + BRIDGE_START_TIMEOUT_MS
  while (!(await portOpen(port))) {
    if (exited) fail(`fathom-devbridge ${exited} before listening (see its log above).`)
    if (Date.now() > deadline) {
      child.kill('SIGKILL')
      fail(`fathom-devbridge did not listen on port ${String(port)} within 180 s.`)
    }
    await new Promise((r) => setTimeout(r, 250))
  }

  process.env[ENV.port] = String(port)
  process.env[ENV.token] = token
  process.env[ENV.pid] = String(child.pid)
  console.log('[live e2e] fathom-devbridge is listening')
}
