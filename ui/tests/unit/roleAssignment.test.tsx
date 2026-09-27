/**
 * 033 US4: roles state reducer, directory filtering (the picker's fake-transport-fed source),
 * and the read-only orphan badge.
 */
import { act, fireEvent, render, screen } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import { CREATE_ROLE_OPTION } from '@/components/rbac/CreateRoleOption'
import type { BindTarget } from '@/components/rbac/RoleForm'
import { RoleAssignmentDialog } from '@/features/roles/RoleAssignmentDialog'
import { filterGroups, filterUsers } from '@/features/roles/directory'
import { OrphanedSubjectBadge } from '@/features/roles/OrphanedSubjectBadge'
import { initialRolesState, reduceRoles } from '@/features/roles/state'
import { ingestCapabilityChecked, resetCapabilities, toQuery } from '@/lib/capabilities'
import type { BackendCmd, SystemEvent } from '@/protocol/types'
import { useStore } from '@/store'

// 042's RoleForm has its own tests; here it stands in for a create that succeeds, so the test
// covers only how the binding flow swaps it in and takes the new role back as its roleRef.
vi.mock('@/components/rbac/RoleForm', () => ({
  RoleForm: (props: { kind: string; onCreated?: (t: BindTarget) => void }) => (
    <button
      data-testid="stub-role-form"
      onClick={() =>
        props.onCreated?.({
          roleKind: props.kind as BindTarget['roleKind'],
          name: 'pod-reader',
          namespace: props.kind === 'Role' ? 'team-a' : null,
        })
      }
    >
      create {props.kind}
    </button>
  ),
}))

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

describe('Assign role: "Create a new role…" (043 US8, FR-041)', () => {
  const sent: BackendCmd[] = []
  const source = { users: [], groups: [], stale: false, error: null }

  function allow(resource: 'roles' | 'clusterroles', namespace: string | null) {
    act(() => {
      ingestCapabilityChecked(
        toQuery('ctx-a', {
          apiGroup: 'rbac.authorization.k8s.io',
          verb: 'create',
          resource,
          namespace,
        }),
        { allowed: true, reason: null, source: 'Review', verified: true },
      )
    })
  }

  beforeEach(() => {
    sent.length = 0
    resetCapabilities()
    useStore.setState((s) => ({
      send: (cmd: BackendCmd) => void sent.push(cmd),
      session: { ...s.session, activeContext: 'ctx-a' },
    }))
  })

  it('offers the option gated on creating roles in that namespace', () => {
    render(<RoleAssignmentDialog namespace="team-a" source={source} onClose={() => {}} />)
    fireEvent.change(screen.getByTestId('role-ref-kind'), { target: { value: 'Role' } })
    const option = screen.getByTestId('role-assignment-create-role')
    expect(option).toHaveTextContent(CREATE_ROLE_OPTION)
    expect(option).toBeDisabled()
    allow('roles', 'team-a')
    expect(option).toBeEnabled()
  })

  it('returns to the binding with the new role as its roleRef', () => {
    render(<RoleAssignmentDialog namespace="team-a" source={source} onClose={() => {}} />)
    fireEvent.change(screen.getByTestId('role-ref-kind'), { target: { value: 'Role' } })
    allow('roles', 'team-a')
    fireEvent.click(screen.getByTestId('role-assignment-create-role'))
    fireEvent.click(screen.getByTestId('stub-role-form'))
    expect(screen.getByTestId('role-ref-kind')).toHaveValue('Role')
    expect(screen.getByTestId('role-ref-name')).toHaveValue('pod-reader')
  })

  it('checks cluster-wide access for a ClusterRole', () => {
    render(<RoleAssignmentDialog namespace={null} source={source} onClose={() => {}} />)
    const option = screen.getByTestId('role-assignment-create-role')
    expect(option).toBeDisabled()
    allow('clusterroles', null)
    expect(option).toBeEnabled()
    fireEvent.click(option)
    expect(screen.getByTestId('stub-role-form')).toHaveTextContent('create ClusterRole')
  })
})
