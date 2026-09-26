import { describe, expect, it } from 'vitest'
import { parse, parseAllDocuments } from 'yaml'

import {
  cleanManifest,
  compareRefs,
  exportFilename,
  renderBundle,
  renderManifest,
  SERVER_ANNOTATIONS,
} from '@/features/manifest-export/clean'

function deployment(): Record<string, unknown> {
  return {
    status: { replicas: 1 },
    spec: { replicas: 1, selector: { matchLabels: { app: 'web' } } },
    metadata: {
      name: 'web',
      namespace: 'default',
      labels: { app: 'web' },
      finalizers: ['keep.example.com'],
      annotations: {
        'kubectl.kubernetes.io/last-applied-configuration': '{}',
        'deployment.kubernetes.io/revision': '3',
        'team.example.com/owner': 'infra',
      },
      managedFields: [{ manager: 'kubectl' }],
      resourceVersion: '123',
      uid: 'abc',
      creationTimestamp: '2026-01-01T00:00:00Z',
      generation: 4,
      selfLink: '/apis/apps/v1/x',
      deletionTimestamp: '2026-01-02T00:00:00Z',
      deletionGracePeriodSeconds: 30,
    },
    kind: 'Deployment',
    apiVersion: 'apps/v1',
  }
}

describe('cleanManifest', () => {
  it('C1 drops status', () => {
    expect(cleanManifest(deployment()).object).not.toHaveProperty('status')
  })

  it('C2 drops server-managed metadata and keeps the rest (FR-009)', () => {
    const md = cleanManifest(deployment()).object.metadata as Record<string, unknown>
    for (const f of [
      'managedFields',
      'resourceVersion',
      'uid',
      'creationTimestamp',
      'generation',
      'selfLink',
      'deletionTimestamp',
      'deletionGracePeriodSeconds',
    ]) {
      expect(md).not.toHaveProperty(f)
    }
    expect(md).toMatchObject({
      name: 'web',
      namespace: 'default',
      labels: { app: 'web' },
      finalizers: ['keep.example.com'],
    })
  })

  it('C3 drops ownerReferences and warns for a controller owner only', () => {
    const pod = {
      apiVersion: 'v1',
      kind: 'Pod',
      metadata: {
        name: 'web-1',
        ownerReferences: [
          { kind: 'ReplicaSet', name: 'web-abc', controller: true },
          { kind: 'ConfigMap', name: 'extra' },
        ],
      },
    }
    const { object, warnings } = cleanManifest(pod)
    expect(object.metadata).not.toHaveProperty('ownerReferences')
    expect(warnings).toHaveLength(1)
    expect(warnings[0]).toContain('ReplicaSet web-abc')
    expect(
      cleanManifest({ metadata: { ownerReferences: [{ kind: 'X', name: 'y' }] } }).warnings,
    ).toEqual([])
  })

  it('C4 drops every server annotation and keeps others; empty annotations go', () => {
    const md = cleanManifest(deployment()).object.metadata as Record<string, unknown>
    expect(md.annotations).toEqual({ 'team.example.com/owner': 'infra' })
    const pvc = {
      metadata: {
        name: 'data',
        annotations: Object.fromEntries(SERVER_ANNOTATIONS.map((a) => [a, 'x'])),
      },
    }
    expect(cleanManifest(pvc).object.metadata).not.toHaveProperty('annotations')
  })

  it('C5 drops Service clusterIP(s) unless headless', () => {
    const svc = (ip: string) => ({
      apiVersion: 'v1',
      kind: 'Service',
      metadata: { name: 's' },
      spec: { clusterIP: ip, clusterIPs: [ip], ports: [{ port: 80 }] },
    })
    const clean = cleanManifest(svc('10.0.0.1')).object.spec as Record<string, unknown>
    expect(clean).not.toHaveProperty('clusterIP')
    expect(clean).not.toHaveProperty('clusterIPs')
    expect(clean.ports).toEqual([{ port: 80 }])
    const headless = cleanManifest(svc('None')).object.spec as Record<string, unknown>
    expect(headless.clusterIP).toBe('None')
    expect(headless.clusterIPs).toEqual(['None'])
  })

  it('C6 drops the generated Job selector and labels unless manualSelector', () => {
    const job = (manual?: boolean) => ({
      apiVersion: 'batch/v1',
      kind: 'Job',
      metadata: { name: 'j' },
      spec: {
        ...(manual !== undefined ? { manualSelector: manual } : {}),
        selector: { matchLabels: { 'controller-uid': 'u' } },
        template: {
          metadata: {
            labels: {
              'controller-uid': 'u',
              'batch.kubernetes.io/controller-uid': 'u',
              'job-name': 'j',
              'batch.kubernetes.io/job-name': 'j',
              app: 'keep',
            },
          },
        },
      },
    })
    const spec = cleanManifest(job()).object.spec as Record<string, unknown>
    expect(spec).not.toHaveProperty('selector')
    expect(spec.template).toEqual({ metadata: { labels: { app: 'keep' } } })
    const manual = cleanManifest(job(true)).object.spec as Record<string, unknown>
    expect(manual).toHaveProperty('selector')
    expect(JSON.stringify(manual.template)).toContain('controller-uid')
  })

  it('C7 puts apiVersion, kind, metadata first', () => {
    expect(Object.keys(cleanManifest(deployment()).object)).toEqual([
      'apiVersion',
      'kind',
      'metadata',
      'spec',
    ])
  })

  it('C8 never mutates the input', () => {
    const input = deployment()
    const before = JSON.stringify(input)
    cleanManifest(input)
    renderManifest(input, { clean: true })
    expect(JSON.stringify(input)).toBe(before)
  })
})

describe('renderManifest', () => {
  it('raw mode keeps everything the cluster returned', () => {
    const { yaml } = renderManifest(JSON.stringify(deployment()), { clean: false })
    expect(yaml).toContain('resourceVersion:')
    expect(yaml).toContain('status:')
    expect(parse(yaml)).toEqual(deployment())
  })

  it('clean mode output starts with apiVersion and parses back', () => {
    const { yaml } = renderManifest(JSON.stringify(deployment()), { clean: true })
    expect(yaml.startsWith('apiVersion: apps/v1\nkind: Deployment\nmetadata:\n')).toBe(true)
    expect(yaml).not.toContain('status:')
  })

  it('FR-013: tricky strings stay strings through YAML 1.1 and 1.2 readers', () => {
    const data = {
      a: 'yes',
      b: 'on',
      c: 'off',
      d: '0755',
      e: '1e3',
      f: 'null',
      g: '~',
      h: '2026-01-01',
      i: 'true',
      j: '12:30',
      k: '',
      l: 'line one\nline two\n',
      m: '0x1F',
      n: '-.inf',
      o: ' leading',
      p: 'a: b',
      q: '# not a comment',
      r: '*star',
      s: '3.14',
    }
    const obj = { apiVersion: 'v1', kind: 'ConfigMap', metadata: { name: 'c' }, data }
    const { yaml } = renderManifest(obj, { clean: true })
    expect(parse(yaml, { version: '1.1' })).toEqual(obj)
    expect(parse(yaml, { version: '1.2' })).toEqual(obj)
  })

  it('long strings are not folded', () => {
    const long = 'x'.repeat(300)
    const { yaml } = renderManifest({ kind: 'ConfigMap', data: { long } }, { clean: false })
    expect(yaml).toContain(`long: ${long}`)
  })

  it('warns that Secret values are redacted in both modes', () => {
    const secret = {
      apiVersion: 'v1',
      kind: 'Secret',
      metadata: { name: 's' },
      data: { k: '<redacted>' },
    }
    expect(renderManifest(secret, { clean: true }).warnings.join(' ')).toContain('redacted')
    expect(renderManifest(secret, { clean: false }).warnings.join(' ')).toContain('redacted')
  })
})

describe('renderBundle', () => {
  it('joins documents with a --- line and ends with one newline', () => {
    const docs = ['a: 1\n', 'b: 2\n', 'c: 3\n']
    const out = renderBundle(docs)
    expect(out).toBe('a: 1\n---\nb: 2\n---\nc: 3\n')
    expect(parseAllDocuments(out).map((d) => d.toJS() as unknown)).toEqual([
      { a: 1 },
      { b: 2 },
      { c: 3 },
    ])
  })

  it('is empty for no documents', () => {
    expect(renderBundle([])).toBe('')
  })

  it('bundles sort by namespace, then name', () => {
    const refs = [
      { namespace: 'b', name: 'x' },
      { namespace: 'a', name: 'z' },
      { namespace: 'a', name: 'y' },
      { namespace: null, name: 'cluster' },
    ]
    expect([...refs].sort(compareRefs).map((r) => `${r.namespace ?? ''}/${r.name}`)).toEqual([
      '/cluster',
      'a/y',
      'a/z',
      'b/x',
    ])
  })
})

describe('exportFilename (FR-005)', () => {
  it('names one namespaced object kind-namespace-name', () => {
    expect(exportFilename('Deployment', [{ namespace: 'default', name: 'web' }])).toBe(
      'deployment-default-web.yaml',
    )
  })
  it('omits the namespace for cluster-scoped objects', () => {
    expect(exportFilename('ClusterRole', [{ namespace: null, name: 'view' }])).toBe(
      'clusterrole-view.yaml',
    )
  })
  it('names a bundle by kind and count', () => {
    expect(
      exportFilename('Deployment', [
        { namespace: 'a', name: 'x' },
        { namespace: 'a', name: 'y' },
        { namespace: 'a', name: 'z' },
      ]),
    ).toBe('deployment-3-objects.yaml')
  })
  it('sanitizes odd characters', () => {
    expect(exportFilename('ClusterRole', [{ name: 'system:controller:foo' }])).toBe(
      'clusterrole-system-controller-foo.yaml',
    )
  })
})
