#!/usr/bin/env node
// Fails when an app defines its own copy of a shared UI kit piece (spec 065, FR-015, research R10).
//
//   node scripts/check-kit-copies.mjs [--root <dir>] [--scan <dir>]... [--allow <file>]
//
// 1. Reads the value export names from packages/ui/src/kit/index.ts. Type-only exports
//    (`type Column`) are skipped: a type carries no markup, and generic names like `Column` would
//    flag every table in the repository.
// 2. Scans each app's source for a top-level declaration of one of those names:
//    `function X`, `class X`, `const|let|var X =`, `export default function X`, and an export
//    clause that renames something into a kit name (`export { Mine as EmptyState }`). Imports and
//    usages never count, and nor do nested declarations or test files (*.test.*, *.spec.*).
// 3. Honours packages/ui/kit-allow.json, a list of `{ "file", "name", "reason" }`, where `file`
//    is relative to the root. An entry with an empty reason fails, and so does an entry that no
//    longer matches anything (a stale entry).
//
// Options (they exist so the unit test can point the check at fixture trees):
//   --root <dir>    the repository root (default: this script's parent directory). The kit index
//                   is read from under it.
//   --scan <dir>    a directory to scan, relative to the root; repeatable. Default: the three apps.
//   --allow <file>  the allow-list, relative to the root. Default: packages/ui/kit-allow.json.
//
// Exit codes: 0 clean, 1 copies or allow-list problems found, 2 bad usage or an unreadable kit.
// Plain Node, no dependencies.

import { existsSync, readdirSync, readFileSync, statSync } from 'node:fs'
import { dirname, join, relative, resolve, sep } from 'node:path'
import { fileURLToPath } from 'node:url'

const KIT_INDEX = 'packages/ui/src/kit/index.ts'
const ALLOW_FILE = 'packages/ui/kit-allow.json'
const DEFAULT_SCAN = ['apps/keel/web/src', 'apps/depths/web/src', 'ui/src']
const SOURCE = /\.(?:[cm]?[jt]sx?)$/
const SKIP_FILE = /\.(?:test|spec)\.[cm]?[jt]sx?$|\.d\.ts$/
const SKIP_DIR = new Set(['node_modules', 'dist', 'generated'])

function parseArgs(argv) {
  let root = resolve(dirname(fileURLToPath(import.meta.url)), '..')
  let allow = ALLOW_FILE
  const scan = []
  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i]
    const value = argv[i + 1]
    if (['--root', '--scan', '--allow'].includes(arg) && value !== undefined) {
      if (arg === '--root') root = resolve(value)
      else if (arg === '--allow') allow = value
      else scan.push(value)
      i++
    } else {
      console.error(`usage: check-kit-copies.mjs [--root <dir>] [--scan <dir>]... [--allow <file>]  (bad argument: ${arg})`)
      process.exit(2)
    }
  }
  return { root, allow, scan: scan.length > 0 ? scan : DEFAULT_SCAN }
}

/** The kit's value export names, from its index's `export { … }` clauses. */
export function kitNames(indexSource) {
  const names = new Set()
  const noComments = indexSource.replace(/\/\*[\s\S]*?\*\//g, '').replace(/\/\/.*$/gm, '')
  for (const clause of noComments.matchAll(/export\s*(type\s*)?\{([^}]*)\}/g)) {
    if (clause[1]) continue
    for (const raw of clause[2].split(',')) {
      const spec = raw.trim()
      if (spec === '' || spec.startsWith('type ')) continue
      const parts = spec.split(/\s+as\s+/)
      names.add(parts[parts.length - 1].trim())
    }
  }
  for (const m of noComments.matchAll(/^export\s+(?:default\s+)?(?:async\s+)?(?:function\*?|class|const|let|var)\s+([A-Za-z_$][\w$]*)/gm)) {
    names.add(m[1])
  }
  return names
}

/** Kit names a file imports under another name (`import { ErrorLine as Alert } from '@companion/ui/kit'`). */
function aliasedKitImports(source) {
  const aliased = new Set()
  for (const m of source.matchAll(/^import\s*(?:type\s*)?\{([^}]*)\}\s*from\s*['"]@companion\/ui\/kit['"]/gm)) {
    for (const raw of m[1].split(',')) {
      const parts = raw.trim().replace(/^type\s+/, '').split(/\s+as\s+/)
      if (parts.length === 2) aliased.add(parts[0].trim())
    }
  }
  return aliased
}

/**
 * Top-level declarations in one file whose name is a kit name: `[{ line, name }]`.
 *
 * Two shapes are not copies, so they are not reported:
 * - an adapter: the file imports the kit piece under another name and declares its own `X` around
 *   it (`import { ErrorLine as Alert } …; function ErrorLine() { return <Alert … /> }`);
 * - a lazy binding of a module (`const Markdown = lazy(() => import('./GuideMarkdown'))`), which is
 *   an import, not a definition.
 */
export function findCopies(source, names) {
  const hits = []
  const lines = source.split('\n')
  const adapters = aliasedKitImports(source)
  const decl = /^(?:export\s+(?:default\s+)?)?(?:declare\s+)?(?:abstract\s+)?(?:async\s+)?(?:function\*?\s+|class\s+|(?:const|let|var)\s+)([A-Za-z_$][\w$]*)/
  const lazy = /^(?:export\s+)?(?:const|let|var)\s+[\w$]+\s*(?::[^=]*)?=\s*(?:React\.)?lazy\s*\(/
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i]
    const m = decl.exec(line)
    if (!m || !names.has(m[1]) || adapters.has(m[1]) || lazy.test(line)) continue
    hits.push({ line: i + 1, name: m[1] })
  }
  // `export { Mine as EmptyState }` or `export { Mine as EmptyState } from './mine'`: renaming
  // something local into a kit name. Re-exports from a package (the kit itself) are fine.
  for (const m of source.matchAll(/^export\s*\{([^}]*)\}(?:\s*from\s*['"]([^'"]+)['"])?/gm)) {
    const from = m[2]
    if (from !== undefined && !from.startsWith('.') && !from.startsWith('@/')) continue
    for (const raw of m[1].split(',')) {
      const parts = raw.trim().split(/\s+as\s+/)
      const exported = parts[parts.length - 1].trim()
      if (parts.length === 2 && !parts[0].startsWith('type ') && names.has(exported)) {
        hits.push({ line: source.slice(0, m.index).split('\n').length, name: exported })
      }
    }
  }
  return hits
}

function* walk(dir) {
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const path = join(dir, entry.name)
    if (entry.isDirectory()) {
      if (!SKIP_DIR.has(entry.name)) yield* walk(path)
    } else if (SOURCE.test(entry.name) && !SKIP_FILE.test(entry.name)) {
      yield path
    }
  }
}

function readAllowList(root, name) {
  const path = resolve(root, name)
  if (!existsSync(path)) return { entries: [], problems: [] }
  let data
  try {
    data = JSON.parse(readFileSync(path, 'utf8'))
  } catch (e) {
    return { entries: [], problems: [`${name}: not valid JSON (${e.message})`] }
  }
  if (!Array.isArray(data)) return { entries: [], problems: [`${name}: must be a JSON array`] }
  const entries = []
  const problems = []
  data.forEach((entry, i) => {
    const at = `${name} entry ${i}`
    if (typeof entry?.file !== 'string' || typeof entry?.name !== 'string') {
      problems.push(`${at}: needs "file" and "name" strings`)
    } else if (typeof entry.reason !== 'string' || entry.reason.trim() === '') {
      problems.push(`${at} (${entry.file} ${entry.name}): needs a non-empty "reason"`)
    } else {
      entries.push({ file: entry.file.split('/').join(sep), name: entry.name, used: false })
    }
  })
  return { entries, problems }
}

function main() {
  const { root, allow, scan } = parseArgs(process.argv.slice(2))
  const indexPath = join(root, KIT_INDEX)
  if (!existsSync(indexPath)) {
    console.error(`check-kit-copies: cannot read the kit index at ${indexPath}`)
    process.exit(2)
  }
  const names = kitNames(readFileSync(indexPath, 'utf8'))
  const { entries, problems } = readAllowList(root, allow)

  const copies = []
  for (const dir of scan) {
    const abs = resolve(root, dir)
    if (!existsSync(abs) || !statSync(abs).isDirectory()) continue
    for (const file of walk(abs)) {
      const rel = relative(root, file)
      for (const hit of findCopies(readFileSync(file, 'utf8'), names)) {
        const allowed = entries.find((e) => e.file === rel && e.name === hit.name)
        if (allowed) allowed.used = true
        else copies.push({ file: rel, ...hit })
      }
    }
  }
  for (const e of entries) {
    if (!e.used) problems.push(`${allow}: stale entry (${e.file} ${e.name}) matches nothing; remove it`)
  }

  const show = (p) => p.split(sep).join('/')
  for (const c of copies) {
    console.error(`${show(c.file)}:${c.line} defines ${c.name}; import it from @companion/ui/kit`)
  }
  for (const p of problems) console.error(show(p))
  if (copies.length > 0 || problems.length > 0) {
    console.error(
      `check-kit-copies: ${copies.length} copies of kit pieces, ${problems.length} allow-list problems. ` +
        `Use the kit's piece, or add { "file", "name", "reason" } to ${allow}.`,
    )
    process.exit(1)
  }
  console.log(`check-kit-copies: no app copies of the ${names.size} kit exports`)
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) main()
