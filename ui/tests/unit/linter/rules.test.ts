import { describe, expect, it } from 'vitest'

import { RULES } from '@/linter/rules'
import { bare, good, input, must, podTemplate } from './helpers'

const rule = (id: string) => must(RULES.find((r) => r.id === id))
const row = (name: string, ns = 'd', extra = {}) => ({ name, namespace: ns, ...extra })

describe('resource-limits', () => {
  it('flags a naked pod and a deployment with missing limits, not a compliant one', () => {
    const out = rule('resource-limits').run(
      input({
        lists: {
          'workloads/deployments': { rows: [row('bad'), row('ok')] },
          'workloads/statefulsets': { rows: [] },
          'workloads/daemonsets': { rows: [] },
          'workloads/pods': { rows: [row('p'), row('owned')] },
        },
        details: {
          'Deployment|d|bad': { raw: podTemplate([bare]) },
          'Deployment|d|ok': { raw: podTemplate([good]) },
          'Pod|d|p': { raw: { metadata: {}, spec: { containers: [bare] } } },
          'Pod|d|owned': { raw: { metadata: { ownerReferences: [{ kind: 'ReplicaSet' }] }, spec: { containers: [bare] } } },
        },
      }),
    )
    expect(out.findings.map((f) => f.ref.name).sort()).toEqual(['bad', 'p'])
    expect(out.checked).toBe(3)
    expect(out.status).toBe('evaluated')
  })
  it('is pending while details are missing and skipped when no lists are readable', () => {
    const pend = rule('resource-limits').run(input({ lists: { 'workloads/deployments': { rows: [row('x')] } } }))
    expect(pend.status).toBe('pending')
    expect(pend.pending).toBe(1)
    expect(rule('resource-limits').run(input({})).status).toBe('skipped')
  })
})

describe('probes', () => {
  it('flags containers missing liveness/readiness', () => {
    const out = rule('probes').run(
      input({
        lists: { 'workloads/deployments': { rows: [row('a'), row('b')] }, 'workloads/statefulsets': { rows: [] }, 'workloads/daemonsets': { rows: [] } },
        details: { 'Deployment|d|a': { raw: podTemplate([bare]) }, 'Deployment|d|b': { raw: podTemplate([good]) } },
      }),
    )
    expect(out.findings).toHaveLength(1)
    expect(must(out.findings[0]).ref.name).toBe('a')
    expect(must(out.findings[0]).message).toContain('startup')
  })
})

describe('naked-pod', () => {
  it('flags only pods with zero ownerReferences', () => {
    const out = rule('naked-pod').run(
      input({
        lists: { 'workloads/pods': { rows: [row('n'), row('o')] } },
        details: {
          'Pod|d|n': { raw: { metadata: {} } },
          'Pod|d|o': { raw: { metadata: { ownerReferences: [{ kind: 'CustomThing', name: 'x' }] } } },
        },
      }),
    )
    expect(out.findings.map((f) => f.ref.name)).toEqual(['n'])
  })
  it('skips when pods are not visible', () => {
    expect(rule('naked-pod').run(input({ lists: { 'workloads/pods': { status: 'error', rows: [] } } })).status).toBe('skipped')
  })
})

describe('rbac-wildcard', () => {
  it('flags wildcard verbs/resources on Roles and ClusterRoles, exempting built-ins', () => {
    const out = rule('rbac-wildcard').run(
      input({
        lists: {
          'access/roles': { rows: [row('r1'), row('r2')] },
          'access/clusterroles': { rows: [{ name: 'cr' }, { name: 'system:node' }, { name: 'cluster-admin' }] },
        },
        details: {
          'Role|d|r1': { raw: {}, extra: { rules: [{ verbs: ['*'], resources: ['pods'] }] } },
          'Role|d|r2': { raw: {}, extra: { rules: [{ verbs: ['get'], resources: ['pods'] }] } },
          'ClusterRole|*|cr': { raw: {}, extra: { rules: [{ verbs: ['get'], resources: ['*'] }] } },
        },
      }),
    )
    expect(out.findings.map((f) => f.ref.name).sort()).toEqual(['cr', 'r1'])
    expect(must(out.findings.find((f) => f.ref.name === 'cr')).ref.namespace).toBeNull()
  })
})

describe('missing-pdb', () => {
  const lists = (pdbs: unknown[]) => ({
    'workloads/deployments': { rows: [row('web', 'd', { desired: 3 }), row('solo', 'd', { desired: 1 })] },
    'workloads/statefulsets': { rows: [] },
    'workloads/replicasets': { rows: [row('rs-owned', 'd', { desired: 2 }), row('rs-free', 'd', { desired: 2 })] },
    'config/poddisruptionbudgets': { rows: pdbs },
  })
  const wl = { 'Deployment|d|web': { raw: podTemplate([good]) } }
  const rs = {
    'ReplicaSet|d|rs-owned': { raw: { ...podTemplate([good], {}, { app: 'x' }), metadata: { ownerReferences: [{ kind: 'Deployment' }] } } },
    'ReplicaSet|d|rs-free': { raw: podTemplate([good], {}, { app: 'y' }) },
  }
  it('flags multi-replica workloads without a selecting PDB', () => {
    const out = rule('missing-pdb').run(input({ lists: lists([]), details: { ...wl, ...rs } }))
    expect(out.findings.map((f) => f.ref.name).sort()).toEqual(['rs-free', 'web'])
  })
  it('accepts a matching PDB and ignores non-matching ones', () => {
    const pdbDetail = (labels: Record<string, string>) => ({ raw: { spec: { selector: { matchLabels: labels } } } })
    const covered = rule('missing-pdb').run(
      input({ lists: lists([row('p')]), details: { ...wl, ...rs, 'PodDisruptionBudget|d|p': pdbDetail({ app: 'web' }) } }),
    )
    expect(covered.findings.map((f) => f.ref.name)).toEqual(['rs-free'])
    const other = rule('missing-pdb').run(
      input({ lists: lists([row('p')]), details: { ...wl, ...rs, 'PodDisruptionBudget|d|p': pdbDetail({ app: 'nope' }) } }),
    )
    expect(other.findings.map((f) => f.ref.name).sort()).toEqual(['rs-free', 'web'])
  })
  it('holds verdicts while PDB details load and skips without PDB visibility', () => {
    const pend = rule('missing-pdb').run(input({ lists: lists([row('p')]), details: { ...wl, ...rs } }))
    expect(pend.findings).toHaveLength(0)
    expect(pend.status).toBe('pending')
    const l = lists([])
    expect(rule('missing-pdb').run(input({ lists: { ...l, 'config/poddisruptionbudgets': { status: 'error', rows: [] } } })).status).toBe('skipped')
  })
})

describe('unused-config', () => {
  it('flags unreferenced ConfigMaps/Secrets and honors env, envFrom, volumes and exemptions', () => {
    const c = {
      name: 'c',
      envFrom: [{ configMapRef: { name: 'cm-env' } }],
      env: [{ name: 'X', valueFrom: { secretKeyRef: { name: 'sec-env' } } }],
    }
    const out = rule('unused-config').run(
      input({
        lists: {
          'config/configmaps': { rows: [row('cm-env'), row('cm-vol'), row('cm-unused'), row('kube-root-ca.crt')] },
          'config/secrets': {
            rows: [
              row('sec-env', 'd', { secret_type: 'Opaque' }),
              row('sec-unused', 'd', { secret_type: 'Opaque' }),
              row('tok', 'd', { secret_type: 'kubernetes.io/service-account-token' }),
            ],
          },
          'workloads/pods': { rows: [row('p')] },
          'workloads/deployments': { rows: [row('w')] },
          'workloads/statefulsets': { rows: [] },
          'workloads/daemonsets': { rows: [] },
        },
        details: {
          'Pod|d|p': { raw: { spec: { containers: [c] } } },
          'Deployment|d|w': { raw: podTemplate([bare], { volumes: [{ configMap: { name: 'cm-vol' } }] }) },
        },
      }),
    )
    expect(out.findings.map((f) => f.ref.name).sort()).toEqual(['cm-unused', 'sec-unused'])
  })
  it('does not report anything until all referencing objects are read', () => {
    const out = rule('unused-config').run(
      input({
        lists: {
          'config/configmaps': { rows: [row('cm')] },
          'workloads/pods': { rows: [row('p')] },
          'workloads/deployments': { rows: [] },
          'workloads/statefulsets': { rows: [] },
          'workloads/daemonsets': { rows: [] },
        },
      }),
    )
    expect(out.findings).toHaveLength(0)
    expect(out.status).toBe('pending')
  })
})

describe('unused-serviceaccount', () => {
  it('is always skipped', () => {
    expect(rule('unused-serviceaccount').run(input({})).status).toBe('skipped')
    expect(rule('unused-serviceaccount').run(input({ lists: { 'access/serviceaccounts': { rows: [row('sa')] } } })).findings).toHaveLength(0)
  })
})

describe('deprecated-api', () => {
  it('flags objects with a deprecated managedFields apiVersion', () => {
    const mf = (apiVersion: string) => ({ metadata: { managedFields: [{ apiVersion }] } })
    const out = rule('deprecated-api').run(
      input({
        lists: { 'workloads/deployments': { rows: [row('old'), row('new')] }, 'config/poddisruptionbudgets': { rows: [row('pdb')] } },
        details: {
          'Deployment|d|old': { raw: mf('extensions/v1beta1') },
          'Deployment|d|new': { raw: mf('apps/v1') },
          'PodDisruptionBudget|d|pdb': { raw: mf('policy/v1beta1') },
        },
      }),
    )
    expect(out.findings.map((f) => f.ref.name).sort()).toEqual(['old', 'pdb'])
    expect(must(out.findings[0]).message).toContain('removed in')
  })
})
