// @vitest-environment node
/** 040 US3 (FR-018): the kind config shown before create is the one the backend passes to kind. */
import { readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'

import { describe, expect, it } from 'vitest'

import { KIND_BARE_CONFIG } from '@/components/panels/kind-validation'

describe('KIND_BARE_CONFIG', () => {
  it('matches BARE_CONFIG in src/backend/kind.rs', () => {
    const rust = readFileSync(
      fileURLToPath(new URL('../../../../src/backend/kind.rs', import.meta.url)),
      'utf8',
    )
    const m = /const BARE_CONFIG: &str =\s*"((?:[^"\\]|\\.)*)";/.exec(rust)
    expect(m).not.toBeNull()
    const value = (m?.[1] ?? '').replace(/\\n/g, '\n').replace(/\\"/g, '"')
    expect(KIND_BARE_CONFIG).toBe(value)
  })
})
