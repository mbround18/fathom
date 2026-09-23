/**
 * 011-rbac-oidc-administration US1 (T013/T018): the Roles descriptor's "Rules" detail section
 * renders a `PolicyRuleSummary[]` into a readable rule table (api groups / resources / verbs).
 *
 * NOTE: placed under `tests/unit/` (not `src/resources/__tests__/`) — see `rbacRoleRef.test.ts`'s
 * header comment for why.
 */
import { render, screen } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import type { PolicyRuleSummary, RoleDetails } from '@/protocol/types'
import { roles } from '@/resources/roles'

const DETAIL: RoleDetails = {
  name: 'pod-reader',
  namespace: 'default',
  creation_timestamp: '2024-01-01T00:00:00Z',
  labels: { team: 'platform' },
  annotations: {},
  rules: [
    { api_groups: [], resources: ['pods'], verbs: ['get', 'list', 'watch'] },
    { api_groups: ['apps'], resources: ['deployments', 'replicasets'], verbs: ['*'] },
  ],
  raw_json: '{}',
}

function rulesBlock() {
  const section = roles.detailSections.find((s) => s.id === 'rules')
  if (!section) throw new Error('roles descriptor has no "rules" detail section')
  return section.render({ detail: DETAIL })
}

describe('roles descriptor: Rules detail section', () => {
  it('renders one row per PolicyRuleSummary', () => {
    const block = rulesBlock()
    expect(block?.type).toBe('list')
    if (block?.type !== 'list') return
    expect(block.items).toHaveLength(2)
    render(<>{block.items}</>)
    expect(screen.getAllByTestId('rbac-role-rule-row')).toHaveLength(2)
  })

  it('renders the core API group as "(core)", not blank', () => {
    const block = rulesBlock()
    if (block?.type !== 'list') throw new Error('expected a list block')
    render(<>{block.items}</>)
    const groups = screen.getAllByTestId('rbac-role-rule-api-groups')
    expect(groups[0]).toHaveTextContent('(core)')
    expect(groups[1]).toHaveTextContent('apps')
  })

  it('joins resources and verbs readably', () => {
    const block = rulesBlock()
    if (block?.type !== 'list') throw new Error('expected a list block')
    render(<>{block.items}</>)
    const resources = screen.getAllByTestId('rbac-role-rule-resources')
    const verbs = screen.getAllByTestId('rbac-role-rule-verbs')
    expect(resources[0]).toHaveTextContent('pods')
    expect(resources[1]).toHaveTextContent('deployments, replicasets')
    expect(verbs[0]).toHaveTextContent('get, list, watch')
    expect(verbs[1]).toHaveTextContent('*')
  })

  it('is null (loading) before detail arrives', () => {
    const section = roles.detailSections.find((s) => s.id === 'rules')
    expect(section?.render({})).toBeNull()
  })

  it('returns an empty list for a role with no rules', () => {
    const empty: RoleDetails = { ...DETAIL, rules: [] as PolicyRuleSummary[] }
    const section = roles.detailSections.find((s) => s.id === 'rules')
    const block = section?.render({ detail: empty })
    expect(block?.type).toBe('list')
    if (block?.type === 'list') expect(block.items).toHaveLength(0)
  })
})
