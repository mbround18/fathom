/**
 * A unified line diff computed in the browser (spec 053), for comparisons the server has not
 * built: the person's draft against the file as it is on disk now (a save conflict), and a kept
 * version against the current file (history). Previews keep the server's own diff.
 *
 * Common lines at both ends are trimmed first; the middle is matched by longest common
 * subsequence while it stays small, and shown as replaced wholesale when it does not.
 */

/** Context lines around each change, as `diff -u` shows. */
const CONTEXT = 3
/** Above this many cells, the middle is not matched line by line. */
const LCS_MAX_CELLS = 4_000_000

type Op = { kind: ' ' | '-' | '+'; line: string }

function splitLines(text: string): string[] {
  if (text === '') return []
  const lines = text.split('\n')
  if (lines.at(-1) === '') lines.pop()
  return lines
}

function middle(a: string[], b: string[]): Op[] {
  const n = a.length
  const m = b.length
  if (n * m > LCS_MAX_CELLS) return [...a.map((line) => ({ kind: '-' as const, line })), ...b.map((line) => ({ kind: '+' as const, line }))]
  // lens[i][j]: LCS length of a[i..] and b[j..], in one flat array.
  const w = m + 1
  const lens = new Uint32Array((n + 1) * w)
  for (let i = n - 1; i >= 0; i--) {
    for (let j = m - 1; j >= 0; j--) {
      lens[i * w + j] = a[i] === b[j] ? (lens[(i + 1) * w + j + 1] ?? 0) + 1 : Math.max(lens[(i + 1) * w + j] ?? 0, lens[i * w + j + 1] ?? 0)
    }
  }
  const out: Op[] = []
  let i = 0
  let j = 0
  while (i < n && j < m) {
    if (a[i] === b[j]) {
      out.push({ kind: ' ', line: a[i] ?? '' })
      i++
      j++
    } else if ((lens[(i + 1) * w + j] ?? 0) >= (lens[i * w + j + 1] ?? 0)) {
      out.push({ kind: '-', line: a[i] ?? '' })
      i++
    } else {
      out.push({ kind: '+', line: b[j] ?? '' })
      j++
    }
  }
  for (; i < n; i++) out.push({ kind: '-', line: a[i] ?? '' })
  for (; j < m; j++) out.push({ kind: '+', line: b[j] ?? '' })
  return out
}

/**
 * `before` → `after` as a unified diff with `--- a/<path>` / `+++ b/<path>` headers, or `''` when
 * the two texts are the same.
 */
export function lineDiff(path: string, before: string, after: string): string {
  if (before === after) return ''
  const a = splitLines(before)
  const b = splitLines(after)
  let head = 0
  while (head < a.length && head < b.length && a[head] === b[head]) head++
  let tail = 0
  while (tail < a.length - head && tail < b.length - head && a[a.length - 1 - tail] === b[b.length - 1 - tail]) tail++
  const ops: Op[] = [
    ...a.slice(0, head).map((line) => ({ kind: ' ' as const, line })),
    ...middle(a.slice(head, a.length - tail), b.slice(head, b.length - tail)),
    ...a.slice(a.length - tail).map((line) => ({ kind: ' ' as const, line })),
  ]
  if (!ops.some((o) => o.kind !== ' ')) {
    // Only a trailing newline differs.
    return [`--- a/${path}`, `+++ b/${path}`, '@@ -1 +1 @@', '\\ The end of the file changed (a final newline)'].join('\n') + '\n'
  }

  const out = [`--- a/${path}`, `+++ b/${path}`]
  let k = 0
  // Line numbers (1-based) in `a` and `b` of ops[k].
  const posA: number[] = []
  const posB: number[] = []
  let la = 1
  let lb = 1
  for (const o of ops) {
    posA.push(la)
    posB.push(lb)
    if (o.kind !== '+') la++
    if (o.kind !== '-') lb++
  }
  while (k < ops.length) {
    while (k < ops.length && ops[k]?.kind === ' ') k++
    if (k >= ops.length) break
    const start = Math.max(0, k - CONTEXT)
    let end = k
    // Extend while the next change is within 2 × CONTEXT of this one.
    let last = k
    while (end < ops.length) {
      if (ops[end]?.kind !== ' ') last = end
      else if (end - last > 2 * CONTEXT) break
      end++
    }
    const stop = Math.min(ops.length, last + CONTEXT + 1)
    const slice = ops.slice(start, stop)
    const countA = slice.filter((o) => o.kind !== '+').length
    const countB = slice.filter((o) => o.kind !== '-').length
    out.push(`@@ -${String(posA[start] ?? 1)},${String(countA)} +${String(posB[start] ?? 1)},${String(countB)} @@`)
    for (const o of slice) out.push(`${o.kind}${o.line}`)
    k = stop
  }
  return out.join('\n') + '\n'
}
