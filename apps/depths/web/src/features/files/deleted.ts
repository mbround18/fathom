/**
 * The files deleted in this session, per folder (spec 053, US5, T063), so the listing can offer
 * them back through their history. There is no protocol message that lists a folder's deleted
 * files, so a name deleted before this session has to be typed.
 */
import { create } from 'zustand'

import { parentPath, pathParts } from './model'

const key = (stack: string, mount: string, dir: string): string => `${stack}\u0000${mount}\u0000${dir}`

export interface DeletedState {
  /** Names of files deleted in this session, per folder, newest first. */
  names: Record<string, string[]>
}

export const useDeleted = create<DeletedState>()(() => ({ names: {} }))

/** Notes that the file `path` was deleted. */
export function noteDeleted(stack: string, mount: string, path: string): void {
  const name = pathParts(path)?.at(-1)
  if (name === undefined) return
  const k = key(stack, mount, parentPath(path))
  useDeleted.setState((s) => ({ names: { ...s.names, [k]: [name, ...(s.names[k] ?? []).filter((n) => n !== name)] } }))
}

/** Forgets a deleted file, once it is restored. */
export function forgetDeleted(stack: string, mount: string, path: string): void {
  const name = pathParts(path)?.at(-1)
  const k = key(stack, mount, parentPath(path))
  useDeleted.setState((s) => ({ names: { ...s.names, [k]: (s.names[k] ?? []).filter((n) => n !== name) } }))
}

const NONE: readonly string[] = []

/** The files deleted in `dir` this session, newest first. */
export function deletedIn(names: DeletedState['names'], stack: string, mount: string, dir: string): readonly string[] {
  return names[key(stack, mount, dir)] ?? NONE
}
