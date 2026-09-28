/**
 * T022: pure YAML-generation logic for RoleBinding/ClusterRoleBinding create + edit, tested
 * without mounting React. `buildRoleBindingManifest`/`patchRoleBindingSubjects` live in
 * `ui/src/components/rbac/roleBindingYaml.ts`. 043 US8 adds the form's "Create a new role…"
 * option (mounted, with 042's RoleForm stubbed) and the palette's role-create request.
 *
 * NOTE: placed under `tests/unit/` (not `src/resources/__tests__/`) to match this repo's vitest
 * `include` (`vitest.config.ts`: `tests/unit/**\/*.test.{ts,tsx}`) and every other resource test
 * in the codebase (e.g. `tests/unit/rbacRoleRef.test.ts`) — see final report for this deviation
 * from the literal path suggested in the task brief.
 */
import { act, fireEvent, render, screen } from '@testing-library/react'
import { createElement } from 'react'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import { CREATE_ROLE_OPTION } from '@/components/rbac/CreateRoleOption'
import { RoleBindingForm } from '@/components/rbac/RoleBindingForm'
import type { BindTarget } from '@/components/rbac/RoleForm'
import {
  ROLE_CREATE_REQUEST_TTL_MS,
  requestRoleCreate,
  takeRoleCreateRequest,
} from '@/components/rbac/roleCreateRequest'
import { ingestCapabilityChecked, resetCapabilities, toQuery } from '@/lib/capabilities'
import type { BackendCmd } from '@/protocol/types'
import { useStore } from '@/store'
import {
  buildRoleBindingManifest,
  parseLabels,
  patchRoleBindingSubjects,
  roleBindingManifestYaml,
} from '@/components/rbac/roleBindingYaml'

// 042's RoleForm has its own tests; the stub stands in for a create that succeeds, so these cover
// only how the binding form swaps it in and takes the new role back as its roleRef (043 US8).
vi.mock('@/components/rbac/RoleForm', () => ({
  RoleForm: (props: { kind: string; onCreated?: (t: BindTarget) => void }) =>
    createElement(
      'button',
      {
        'data-testid': 'stub-role-form',
        onClick: () =>
          props.onCreated?.({
            roleKind: props.kind as BindTarget['roleKind'],
            name: 'pod-reader',
            namespace: props.kind === 'Role' ? 'team-a' : null,
          }),
      },
      `create ${props.kind}`,
    ),
}))

describe('buildRoleBindingManifest', () => {
  it('maps a RoleBinding: apiVersion/kind/metadata.name+namespace/roleRef/subjects', () => {
    const manifest = buildRoleBindingManifest({
      kind: 'RoleBinding',
      name: 'view-secrets',
      namespace: 'team-a',
      roleRefKind: 'Role',
      roleRefName: 'secret-reader',
      subjects: [
        { kind: 'User', name: 'alice', namespace: null },
        { kind: 'ServiceAccount', name: 'ci-bot', namespace: 'team-a' },
      ],
    })

    expect(manifest).toMatchObject({
      apiVersion: 'rbac.authorization.k8s.io/v1',
      kind: 'RoleBinding',
      metadata: { name: 'view-secrets', namespace: 'team-a' },
      roleRef: { apiGroup: 'rbac.authorization.k8s.io', kind: 'Role', name: 'secret-reader' },
      subjects: [
        { kind: 'User', name: 'alice' },
        { kind: 'ServiceAccount', name: 'ci-bot', namespace: 'team-a' },
      ],
    })
    // User subject carries no namespace field (matches SubjectSummary's `namespace: null`
    // for non-ServiceAccount kinds by omission, mirroring a real manifest).
    const subjects = manifest.subjects as Array<Record<string, unknown>>
    expect(subjects[0]).not.toHaveProperty('namespace')
  })

  it('maps a ClusterRoleBinding: no metadata.namespace, roleRef.kind is ClusterRole', () => {
    const manifest = buildRoleBindingManifest({
      kind: 'ClusterRoleBinding',
      name: 'cluster-admins',
      namespace: null,
      roleRefKind: 'ClusterRole',
      roleRefName: 'cluster-admin',
      subjects: [{ kind: 'Group', name: 'platform-admins', namespace: null }],
    })

    expect(manifest).toMatchObject({
      apiVersion: 'rbac.authorization.k8s.io/v1',
      kind: 'ClusterRoleBinding',
      metadata: { name: 'cluster-admins' },
      roleRef: {
        apiGroup: 'rbac.authorization.k8s.io',
        kind: 'ClusterRole',
        name: 'cluster-admin',
      },
      subjects: [{ kind: 'Group', name: 'platform-admins' }],
    })
    expect(manifest.metadata).not.toHaveProperty('namespace')
  })

  it('renders to YAML containing the roleRef and subject fields', () => {
    const yaml = roleBindingManifestYaml({
      kind: 'RoleBinding',
      name: 'view-secrets',
      namespace: 'team-a',
      roleRefKind: 'Role',
      roleRefName: 'secret-reader',
      subjects: [{ kind: 'User', name: 'alice', namespace: null }],
    })
    expect(yaml).toContain('kind: RoleBinding')
    expect(yaml).toContain('name: secret-reader')
    expect(yaml).toContain('name: alice')
  })
})

describe('patchRoleBindingSubjects (edit mode)', () => {
  const rawJson = JSON.stringify({
    apiVersion: 'rbac.authorization.k8s.io/v1',
    kind: 'RoleBinding',
    metadata: {
      name: 'view-secrets',
      namespace: 'team-a',
      resourceVersion: '42',
      labels: { team: 'platform' },
    },
    roleRef: { apiGroup: 'rbac.authorization.k8s.io', kind: 'Role', name: 'secret-reader' },
    subjects: [{ kind: 'User', name: 'alice' }],
  })

  it('changes only roleRef/subjects, leaving metadata (including resourceVersion) untouched', () => {
    const yaml = patchRoleBindingSubjects(rawJson, 'Role', 'secret-reader', [
      { kind: 'User', name: 'alice', namespace: null },
      { kind: 'User', name: 'bob', namespace: null },
    ])
    expect(yaml).toContain('name: bob')
    expect(yaml).toContain('resourceVersion: "42"')
    expect(yaml).toContain('team: platform')
  })

  it('falls back to an empty base object on unparsable rawJson rather than throwing', () => {
    expect(() => patchRoleBindingSubjects('not json', 'Role', 'x', [])).not.toThrow()
  })
})

describe('labels (create mode)', () => {
  it('parseLabels: blank is empty, key=value pairs parse, whitespace is trimmed', () => {
    expect(parseLabels('')).toEqual({ ok: true, labels: {} })
    expect(parseLabels(' app.kubernetes.io/managed-by = fathom-rbac-setup , tier=a,')).toEqual({
      ok: true,
      labels: { 'app.kubernetes.io/managed-by': 'fathom-rbac-setup', tier: 'a' },
    })
    expect(parseLabels('empty=')).toEqual({ ok: true, labels: { empty: '' } })
  })

  it('parseLabels: rejects entries without =, bad keys, bad values', () => {
    expect(parseLabels('novalue').ok).toBe(false)
    expect(parseLabels('=v').ok).toBe(false)
    expect(parseLabels('bad key=v').ok).toBe(false)
    expect(parseLabels('k=has space').ok).toBe(false)
    expect(parseLabels(`k=${'x'.repeat(64)}`).ok).toBe(false)
  })

  it('buildRoleBindingManifest: labels land in metadata.labels, omitted when empty', () => {
    const base = {
      kind: 'ClusterRoleBinding' as const,
      name: 'fathom-agent-view',
      namespace: null,
      roleRefKind: 'ClusterRole' as const,
      roleRefName: 'view',
      subjects: [{ kind: 'ServiceAccount', name: 'fathom-agent', namespace: 'fathom-e2e' }],
    }
    const labelled = buildRoleBindingManifest({
      ...base,
      labels: { 'app.kubernetes.io/managed-by': 'fathom-rbac-setup' },
    })
    expect(labelled.metadata).toEqual({
      name: 'fathom-agent-view',
      labels: { 'app.kubernetes.io/managed-by': 'fathom-rbac-setup' },
    })
    expect(buildRoleBindingManifest({ ...base, labels: {} }).metadata).not.toHaveProperty('labels')
    expect(buildRoleBindingManifest(base).metadata).not.toHaveProperty('labels')
  })
})

describe('RoleBinding form: "Create a new role…" (043 US8, FR-041)', () => {
  const sent: BackendCmd[] = []

  function allowRoles(namespace: string) {
    act(() => {
      ingestCapabilityChecked(
        toQuery('ctx-a', {
          apiGroup: 'rbac.authorization.k8s.io',
          verb: 'create',
          resource: 'roles',
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

  function mount() {
    render(
      createElement(RoleBindingForm, {
        mode: 'create',
        kind: 'RoleBinding',
        initial: { name: 'readers', namespace: 'team-a', roleRefKind: 'Role' },
        onClose: () => {},
      }),
    )
  }

  it('offers the option under the role picker, gated on creating roles there', () => {
    mount()
    const option = screen.getByTestId('rbac-binding-create-role')
    expect(option).toHaveTextContent(CREATE_ROLE_OPTION)
    expect(option).toBeDisabled()
    allowRoles('team-a')
    expect(option).toBeEnabled()
  })

  it('opens the role form and comes back with the new role as roleRef', () => {
    mount()
    allowRoles('team-a')
    fireEvent.click(screen.getByTestId('rbac-binding-create-role'))
    expect(screen.queryByTestId('rbac-binding-roleref-name')).toBeNull()
    fireEvent.click(screen.getByTestId('stub-role-form'))
    expect(screen.getByTestId('rbac-binding-roleref-kind')).toHaveValue('Role')
    expect(screen.getByTestId('rbac-binding-roleref-name')).toHaveValue('pod-reader')
    // The binding keeps what was already filled in.
    expect(screen.getByTestId('rbac-binding-name')).toHaveValue('readers')
  })
})

describe('palette role-create requests (043 US8, FR-042)', () => {
  it('are taken once, for their own kind, while fresh', () => {
    requestRoleCreate('Role')
    expect(takeRoleCreateRequest('ClusterRole')).toBe(false)
    expect(takeRoleCreateRequest('Role')).toBe(true)
    expect(takeRoleCreateRequest('Role')).toBe(false)
  })

  it('are dropped once stale', () => {
    requestRoleCreate('ClusterRole')
    expect(takeRoleCreateRequest('ClusterRole', Date.now() + ROLE_CREATE_REQUEST_TTL_MS + 1)).toBe(
      false,
    )
  })
})
