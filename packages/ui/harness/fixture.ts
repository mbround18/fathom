import type { ReactNode } from 'react'

/** One kit piece's harness fixture: `harness/pieces/<id>.tsx` (contracts/harness.md). */
export interface Fixture {
  id: string
  /** Each state the piece's spec visits, by name. */
  states: Readonly<Record<string, () => ReactNode>>
}
