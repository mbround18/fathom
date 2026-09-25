/**
 * T022: pure YAML-generation logic for RoleBinding/ClusterRoleBinding create + edit, tested
 * without mounting React. `buildRoleBindingManifest`/`patchRoleBindingSubjects` live in
 * `ui/src/components/rbac/roleBindingYaml.ts`.
 *
 * NOTE: placed under `tests/unit/` (not `src/resources/__tests__/`) to match this repo's vitest
 * `include` (`vitest.config.ts`: `tests/unit/**\/*.test.{ts,tsx}`) and every other resource test
 * in the codebase (e.g. `tests/unit/rbacRoleRef.test.ts`) — see final report for this deviation
 * from the literal path suggested in the task brief.
 */
import { describe, expect, it } from 'vitest'

import {
  buildRoleBindingManifest,
  patchRoleBindingSubjects,
  roleBindingManifestYaml,
} from '@/components/rbac/roleBindingYaml'

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
