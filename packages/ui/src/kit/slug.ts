/**
 * Heading anchors for the guides (spec 063 R7). The same rule runs in `Markdown`, which gives
 * each `##`/`###` of a guide its `id`, and in an app's guide-link tests, which check that every
 * `GuideRef` in the app points at a heading that exists. It matches GitHub's anchors for the
 * guides' plain headings, so `hosts.md#enroll-a-host` works in the repository too.
 */

/** `"What's a binding?"` → `whats-a-binding`. */
export function slug(text: string): string {
  return text
    .trim()
    .toLowerCase()
    .replace(/[^\p{L}\p{N} -]/gu, '')
    .replace(/ /g, '-')
    .replace(/-{2,}/g, '-')
    .replace(/^-|-$/g, '')
}

/** A slug function for one page: a repeated heading gets `-2`, `-3` and so on. */
export function slugger(): (text: string) => string {
  const seen = new Map<string, number>()
  return (text) => {
    const base = slug(text)
    const n = (seen.get(base) ?? 0) + 1
    seen.set(base, n)
    return n === 1 ? base : `${base}-${String(n)}`
  }
}
