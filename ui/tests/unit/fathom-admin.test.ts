import { describe, expect, it } from 'vitest'

import { isFathomAdmin } from '@/features/identity/state'
import type { OidcIdentity } from '@/protocol/types'

const me = (groups: string[]) => ({ groups }) as unknown as OidcIdentity

describe('isFathomAdmin', () => {
  it('treats an empty admins group as bootstrap: anyone is admin', () => {
    expect(isFathomAdmin(undefined, { admins_group: '' })).toBe(true)
    expect(isFathomAdmin(me([]), { admins_group: '' })).toBe(true)
  })
  it('requires membership once an admins group is set', () => {
    expect(isFathomAdmin(undefined, { admins_group: 'adm' })).toBe(false)
    expect(isFathomAdmin(me(['x']), { admins_group: 'adm' })).toBe(false)
    expect(isFathomAdmin(me(['x', 'adm']), { admins_group: 'adm' })).toBe(true)
  })
})
