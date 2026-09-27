/**
 * 042-role-create: pure Role/ClusterRole form logic (`ui/src/components/rbac/roleYaml.ts`).
 */
import { describe, expect, it } from 'vitest'

import {
  applyPreset,
  buildRoleManifest,
  emptyRule,
  escalationMissing,
  groupOptions,
  isEscalationError,
  parseTokens,
  patchRoleRules,
  presetOf,
  resourceOptions,
  roleManifestYaml,
  roleNameProblem,
  roleWarnings,
  ruleProblems,
  rulesFromRawJson,
  rulesValid,
  toggleVerb,
  type RuleInput,
} from '@/components/rbac/roleYaml'
import type { ApiResourceCatalog } from '@/protocol/types'

const podsReader: RuleInput = {
  apiGroups: [''],
  resources: ['pods', 'pods/log'],
  verbs: ['get', 'list', 'watch'],
  resourceNames: [],
  nonResourceURLs: [],
}

describe('buildRoleManifest', () => {
  it('builds a namespaced Role with the core group as an empty string', () => {
    expect(
      buildRoleManifest({
        kind: 'Role',
        name: 'pod-reader',
        namespace: 'dev',
        rules: [podsReader],
      }),
    ).toEqual({
      apiVersion: 'rbac.authorization.k8s.io/v1',
      kind: 'Role',
      metadata: { name: 'pod-reader', namespace: 'dev' },
      rules: [
        { apiGroups: [''], resources: ['pods', 'pods/log'], verbs: ['get', 'list', 'watch'] },
      ],
    })
  })

  it('omits the namespace for a ClusterRole and keeps resourceNames', () => {
    const m = buildRoleManifest({
      kind: 'ClusterRole',
      name: 'cm-one',
      namespace: 'ignored',
      rules: [{ ...podsReader, resources: ['configmaps'], resourceNames: ['app-config'] }],
    })
    expect(m.metadata).toEqual({ name: 'cm-one' })
    expect((m.rules as unknown[])[0]).toEqual({
      apiGroups: [''],
      resources: ['configmaps'],
      resourceNames: ['app-config'],
      verbs: ['get', 'list', 'watch'],
    })
  })

  it('emits a non-resource URL rule without apiGroups/resources', () => {
    const m = buildRoleManifest({
      kind: 'ClusterRole',
      name: 'health',
      namespace: null,
      rules: [
        { ...emptyRule('custom'), apiGroups: [], verbs: ['get'], nonResourceURLs: ['/healthz'] },
      ],
    })
    expect((m.rules as unknown[])[0]).toEqual({ nonResourceURLs: ['/healthz'], verbs: ['get'] })
  })

  it('renders YAML with the core group quoted', () => {
    const yaml = roleManifestYaml({
      kind: 'Role',
      name: 'r',
      namespace: 'dev',
      rules: [podsReader],
    })
    expect(yaml).toContain('kind: Role')
    expect(yaml).toContain('namespace: dev')
    expect(yaml).toMatch(/apiGroups:\n\s+- ["']{2}/)
  })
})

describe('presets', () => {
  it('read-only and edit are recognised; anything else is custom', () => {
    expect(presetOf([podsReader])).toBe('read-only')
    expect(presetOf(applyPreset([podsReader], 'edit'))).toBe('edit')
    expect(presetOf([toggleVerb(podsReader, 'create', true)])).toBe('custom')
    expect(presetOf([])).toBe('custom')
  })

  it('applyPreset rewrites every rule, custom leaves them alone', () => {
    const two = [podsReader, { ...podsReader, resources: ['services'] }]
    expect(applyPreset(two, 'edit').every((r) => r.verbs.length === 8)).toBe(true)
    expect(applyPreset(two, 'custom')).toEqual(two)
  })

  it('toggleVerb keeps checkbox order, then extra verbs', () => {
    const r = toggleVerb({ ...podsReader, verbs: ['*', 'watch'] }, 'get', true)
    expect(r.verbs).toEqual(['get', 'watch', '*'])
    expect(toggleVerb(r, 'watch', false).verbs).toEqual(['get', '*'])
  })
})

describe('parseTokens', () => {
  it('splits on commas/whitespace and maps core aliases to the empty group', () => {
    expect(parseTokens('pods, services  deployments')).toEqual(['pods', 'services', 'deployments'])
    expect(parseTokens('core,apps,""', true)).toEqual(['', 'apps', ''])
    expect(parseTokens('core')).toEqual(['core'])
  })
})

describe('edit mode', () => {
  const raw = JSON.stringify({
    apiVersion: 'rbac.authorization.k8s.io/v1',
    kind: 'ClusterRole',
    metadata: { name: 'agg', resourceVersion: '42', labels: { a: 'b' } },
    aggregationRule: { clusterRoleSelectors: [{ matchLabels: { x: 'y' } }] },
    rules: [
      { apiGroups: ['apps'], resources: ['deployments'], resourceNames: ['web'], verbs: ['get'] },
      { nonResourceURLs: ['/metrics'], verbs: ['get'] },
    ],
  })

  it('reads every rule field back, including resourceNames and nonResourceURLs', () => {
    expect(rulesFromRawJson(raw)).toEqual([
      {
        apiGroups: ['apps'],
        resources: ['deployments'],
        verbs: ['get'],
        resourceNames: ['web'],
        nonResourceURLs: [],
      },
      {
        apiGroups: [],
        resources: [],
        verbs: ['get'],
        resourceNames: [],
        nonResourceURLs: ['/metrics'],
      },
    ])
    expect(rulesFromRawJson('not json')).toEqual([])
  })

  it('patches only rules and carries metadata/aggregationRule through', () => {
    const yaml = patchRoleRules(raw, 'ClusterRole', [podsReader])
    expect(yaml).toContain('resourceVersion: "42"')
    expect(yaml).toContain('aggregationRule:')
    expect(yaml).toContain('- pods/log')
    expect(yaml).not.toContain('deployments')
  })

  it('warns on system: names and aggregated ClusterRoles', () => {
    expect(roleWarnings('system:controller:foo')).toEqual({ system: true, aggregated: false })
    expect(roleWarnings('agg', raw)).toEqual({ system: false, aggregated: true })
    expect(roleWarnings('plain', '{"rules":[]}')).toEqual({ system: false, aggregated: false })
  })
})

describe('validation', () => {
  it('requires verbs and resources (or non-resource URLs on a ClusterRole)', () => {
    expect(ruleProblems(podsReader, 'Role')).toEqual([])
    expect(ruleProblems({ ...podsReader, verbs: [] }, 'Role')).toContain('Pick at least one verb.')
    expect(ruleProblems({ ...podsReader, resources: [] }, 'Role')).toContain(
      'Add at least one resource.',
    )
    expect(ruleProblems({ ...podsReader, apiGroups: [] }, 'Role').length).toBe(1)
    const nonRes = { ...emptyRule('read-only'), apiGroups: [], nonResourceURLs: ['/healthz'] }
    expect(ruleProblems(nonRes, 'ClusterRole')).toEqual([])
    expect(ruleProblems(nonRes, 'Role').length).toBeGreaterThan(0)
    expect(rulesValid([], 'Role')).toBe(false)
    expect(rulesValid([podsReader], 'Role')).toBe(true)
  })

  it('validates role names as path segments', () => {
    expect(roleNameProblem('pod-reader')).toBeNull()
    expect(roleNameProblem('system:custom')).toBeNull()
    expect(roleNameProblem('')).not.toBeNull()
    expect(roleNameProblem('a/b')).not.toBeNull()
    expect(roleNameProblem('..')).not.toBeNull()
    expect(roleNameProblem(' x')).not.toBeNull()
  })
})

describe('escalation errors', () => {
  const msg =
    'roles.rbac.authorization.k8s.io "x" is forbidden: user "u" (groups=["g"]) is attempting to grant RBAC permissions not currently held:\n{APIGroups:[""], Resources:["secrets"], Verbs:["get"]}\n{NonResourceURLs:["/metrics"], Verbs:["get"]}'

  it('recognises the API server message and lists what is missing', () => {
    expect(isEscalationError(msg)).toBe(true)
    expect(isEscalationError('conflict: resourceVersion changed')).toBe(false)
    expect(isEscalationError(undefined)).toBe(false)
    expect(escalationMissing(msg)).toEqual([
      '{APIGroups:[""], Resources:["secrets"], Verbs:["get"]}',
      '{NonResourceURLs:["/metrics"], Verbs:["get"]}',
    ])
  })
})

describe('discovery pickers', () => {
  const catalog: ApiResourceCatalog = {
    groups: [
      {
        group: '',
        resources: [
          { name: 'nodes', namespaced: false, verbs: ['get'] },
          { name: 'pods', namespaced: true, verbs: ['get'] },
        ],
      },
      { group: 'apps', resources: [{ name: 'deployments', namespaced: true, verbs: ['get'] }] },
      { group: 'example.com', resources: [{ name: 'widgets', namespaced: true, verbs: ['get'] }] },
    ],
    failed_groups: [],
  }

  it('lists every discovered group', () => {
    expect(groupOptions(catalog)).toEqual(['', 'apps', 'example.com'])
    expect(groupOptions(undefined)).toEqual([])
  })

  it('filters resources by selected groups and hides cluster-scoped ones for a Role', () => {
    expect(resourceOptions(catalog, [''], 'Role')).toEqual(['pods'])
    expect(resourceOptions(catalog, [''], 'ClusterRole')).toEqual(['nodes', 'pods'])
    expect(resourceOptions(catalog, ['example.com'], 'Role')).toEqual(['widgets'])
    expect(resourceOptions(catalog, ['*'], 'Role')).toEqual(['deployments', 'pods', 'widgets'])
    expect(resourceOptions(catalog, [], 'ClusterRole')).toHaveLength(4)
  })
})
