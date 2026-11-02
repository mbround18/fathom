import type { Fixture } from '../fixture'

/** Every `<id>.tsx` beside this file is a piece's fixture; none needs registering here. */
const modules = import.meta.glob<{ default: Fixture }>('./*.tsx', { eager: true })

export const PIECES: readonly Fixture[] = Object.values(modules)
  .map((m) => m.default)
  .sort((a, b) => a.id.localeCompare(b.id))
