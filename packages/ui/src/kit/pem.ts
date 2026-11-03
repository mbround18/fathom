/**
 * Reading PEM text without trusting it: which blocks it holds, and whether each is whole. Nothing
 * here decodes a key or a certificate; that is the server's job.
 */

export interface PemBlock {
  /** The label between BEGIN and END: "CERTIFICATE", "PRIVATE KEY". */
  type: string
}

export type PemCheck = { ok: true; blocks: PemBlock[] } | { ok: false; error: string }

const BLOCK = /-----BEGIN ([A-Z0-9 ]+)-----\r?\n([\s\S]*?)-----END ([A-Z0-9 ]+)-----/g
const BASE64_LINE = /^[A-Za-z0-9+/=]*$/
/** Headers some old keys carry ("Proc-Type: 4,ENCRYPTED") before the base64. */
const HEADER = /^[A-Za-z-]+: /

/** The blocks in `text`, or why it is not PEM. `expect` lists the block types the field takes. */
export function checkPem(text: string, expect?: readonly string[]): PemCheck {
  const trimmed = text.trim()
  if (trimmed === '') return { ok: false, error: 'Paste or load a PEM file.' }
  const blocks: PemBlock[] = []
  let rest = trimmed
  for (const m of trimmed.matchAll(BLOCK)) {
    const [whole, begin = '', body = '', end = ''] = m
    if (begin !== end) return { ok: false, error: `A ${begin} block ends as ${end}.` }
    const lines = body.split(/\r?\n/).filter((l) => l.trim() !== '' && !HEADER.test(l))
    if (lines.length === 0 || !lines.every((l) => BASE64_LINE.test(l.trim()))) {
      return { ok: false, error: `The ${begin} block is damaged: its body is not base64.` }
    }
    blocks.push({ type: begin })
    rest = rest.replace(whole, '')
  }
  if (blocks.length === 0) {
    return { ok: false, error: trimmed.includes('-----BEGIN') ? 'A block has no END line.' : 'This is not PEM: it has no BEGIN line.' }
  }
  if (rest.includes('-----BEGIN') || rest.includes('-----END')) return { ok: false, error: 'A block is cut short.' }
  if (expect) {
    const wrong = blocks.find((b) => !expect.includes(b.type))
    if (wrong) return { ok: false, error: `Expected ${expect.join(' or ')}, found ${wrong.type}.` }
  }
  return { ok: true, blocks }
}

/** "2 blocks: CERTIFICATE, CERTIFICATE". */
export function describePem(blocks: readonly PemBlock[]): string {
  const n = blocks.length
  return `${String(n)} ${n === 1 ? 'block' : 'blocks'}: ${blocks.map((b) => b.type).join(', ')}`
}
