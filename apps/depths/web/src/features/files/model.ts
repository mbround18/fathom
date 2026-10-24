/**
 * The volume explorer's pure model (spec 053): what `/api/v1/me` says about it, and its URLs.
 *
 * URLs (FR-024) name the stack, the mount and the path, so a folder or file can be bookmarked and
 * reopened after a reload: `/stacks/<stack>/files` (the mount list), `/stacks/<stack>/files/<mount>`
 * (its root) and `/stacks/<stack>/files/<mount>/<a>/<b>` (a folder or file inside it). Each part
 * is percent-encoded on its own, so a name holding `%`, `#` or `?` survives.
 *
 * A path here is the mount-relative path the protocol carries: components joined by `/`, `''` for
 * the root, no leading slash. The client never resolves `..`; a URL holding one is refused, as the
 * server refuses it.
 */
import type { FileLimits, FilesMode, Me } from '@/protocol/generated'

export type { FileLimits, FilesMode }

/** The explorer's mode for this person; `off` when the server says nothing. */
export function filesMode(me: Me | null): FilesMode {
  return me?.features.files ?? 'off'
}

/** Admins only: why the server forced files off, when it did. */
export function filesReason(me: Me | null): string | null {
  return me?.features.files_reason ?? null
}

export function filesLimits(me: Me | null): FileLimits | null {
  return me?.limits?.files ?? null
}

/**
 * Whether the stack page shows a Files tab: the feature is on (read-only or full) and the person
 * may edit, the same switch as the Edit button. The server gates every command regardless.
 */
export function showFilesTab(me: Me | null): boolean {
  return (me?.features.edit ?? false) && filesMode(me) !== 'off'
}

/** Where the explorer is: a mount (`null` for the mount list) and a path inside it. */
export interface FilesLocation {
  mount: string | null
  path: string
}

export const MOUNT_LIST: FilesLocation = { mount: null, path: '' }

/**
 * A mount-relative path's components: empty and `.` parts dropped, as the server's `RelPath`
 * does. `null` for a `..` part or one holding a NUL.
 */
export function pathParts(path: string): string[] | null {
  const parts = path.split('/').filter((p) => p !== '' && p !== '.')
  return parts.every((p) => p !== '..' && !p.includes('\0')) ? parts : null
}

/** The URL of `loc` in `stack`'s Files tab. */
export function filesHref(stack: string, loc: FilesLocation = MOUNT_LIST): string {
  const base = `/stacks/${encodeURIComponent(stack)}/files`
  if (loc.mount === null) return base
  const parts = pathParts(loc.path) ?? []
  return [base, loc.mount, ...parts].map((p, i) => (i === 0 ? p : encodeURIComponent(p))).join('/')
}

/**
 * The location in the rest of a Files URL (the router's `*`: still encoded, joined by `/`).
 * `null` when it cannot be a location: a part that does not decode, decodes to a `/` or NUL, or
 * is `..`.
 */
export function parseFilesRest(rest: string): FilesLocation | null {
  const raw = rest.split('/').filter(Boolean)
  const parts: string[] = []
  for (const r of raw) {
    let p: string
    try {
      p = decodeURIComponent(r)
    } catch {
      return null
    }
    if (p === '..' || p.includes('/') || p.includes('\0')) return null
    if (p !== '.') parts.push(p)
  }
  const [mount, ...path] = parts
  return mount === undefined ? MOUNT_LIST : { mount, path: path.join('/') }
}

/** The folder holding `path` (the root's parent is the root). */
export function parentPath(path: string): string {
  const parts = pathParts(path) ?? []
  return parts.slice(0, -1).join('/')
}

/** `path` with `name` added; `name` must be one component. */
export function joinPath(path: string, name: string): string {
  return [...(pathParts(path) ?? []), name].join('/')
}
