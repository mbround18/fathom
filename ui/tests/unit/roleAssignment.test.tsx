/**
 * 033 US4: roles state reducer, directory filtering (the picker's fake-transport-fed source),
 * and the read-only orphan badge.
 */
import { render, screen } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import { filterGroups, filterUsers } from '@/features/roles/directory'
import { OrphanedSubjectBadge } from '@/features/roles/OrphanedSubjectBadge'
import { initialRolesState, reduceRoles } from '@/features/roles/state'
import type { SystemEvent } from '@/protocol/types'
import { useStore } from '@/store'

describe('roles reducer', () => {
  it('stores directory snapshots per issuer and preview replies with a rising seq', () => {
    const s = initialRolesState()
    reduceRoles(s, {
      type: 'IdpDirectoryUpdated',
      data: {
        issuer_id: 'r',
        users: [],
        groups: [{ name: 'g', member_count: 1 }],
        fetched_at: 1,
        stale: false,
        error: null,
      },
    })
    expect(s.directory.r?.groups).toHaveLength(1)
    const ev: SystemEvent = {
      type: 'RoleAssignmentPreview',
      data: { yaml: 'y', rendered_subject_name: 'oidc:g', warnings: ['w'] },
    }
    reduceRoles(s, ev)
    reduceRoles(s, ev)
    expect(s.preview).toMatchObject({ seq: 2, renderedSubjectName: 'oidc:g', warnings: ['w'] })
  })
})

describe('directory filters', () => {
  it('filters groups and hides disabled users', () => {
    expect(
      filterGroups(
        [
          { name: 'Dev:a/b', member_count: 1 },
          { name: 'ops', member_count: 1 },
        ],
        'dev',
      ),
    ).toHaveLength(1)
    const users = [
      { sub: '1', username: 'alice', email: null, groups: [], enabled: true },
      { sub: '2', username: 'alina', email: null, groups: [], enabled: false },
    ]
    expect(filterUsers(users, 'ali').map((u) => u.username)).toEqual(['alice'])
  })
})

describe('OrphanedSubjectBadge', () => {
  it('renders only for bindings with orphans', () => {
    useStore.setState((s) => ({
      roles: {
        ...s.roles,
        orphans: [{ binding: 'ClusterRoleBinding/x', subject_name: 'oidc:gone' }],
      },
    }))
    const { rerender } = render(<OrphanedSubjectBadge binding="ClusterRoleBinding/x" />)
    expect(screen.getByTestId('orphaned-subject-badge').textContent).toContain('oidc:gone')
    rerender(<OrphanedSubjectBadge binding="ClusterRoleBinding/y" />)
    expect(screen.queryByTestId('orphaned-subject-badge')).toBeNull()
  })
})
