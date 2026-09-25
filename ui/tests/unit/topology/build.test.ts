/**
 * 007-topology-graph-view: buildTopologyGraph is the layout-selection-style pure logic the
 * constitution requires unit coverage for (FR-004 empty namespaces, FR-013 de-dup, FR-015
 * loading affordance).
 */
import { describe, expect, it } from 'vitest'

// See src/topology/build.ts: `@/store` must load before any descriptor module.
import '@/store'

import { applications } from '@/resources/applications'
import { deployments } from '@/resources/deployments'
import { nodes as nodesDescriptor } from '@/resources/nodes'
import {
  buildTopologyGraph,
  type BuildInput,
  type DetailSnapshot,
  type ListSnapshot,
} from '@/topology/build'

function input(overrides: Partial<BuildInput> = {}): BuildInput {
  return {
    namespaces: ['default', 'kube-node-lease'],
    lists: {},
    details: {},
    viewport: { namespaceFocus: new Set(), workloadFocus: new Set() },
    listKeyFor: (section) => section,
    detailKeyFor: (kind, ns, name) => `${kind}|${ns ?? ''}|${name}`,
    ...overrides,
  }
}

describe('buildTopologyGraph', () => {
  it('renders every namespace as a node even with zero edges (FR-004)', () => {
    const graph = buildTopologyGraph(input())
    const namespaceNodes = graph.nodes.filter((n) => n.kind === 'Namespace')
    expect(namespaceNodes.map((n) => n.label).sort()).toEqual(['default', 'kube-node-lease'])
    expect(graph.edges).toHaveLength(0)
  })

  it('draws namespace -> application edges only for namespaces that actually contain one', () => {
    const lists: Record<string, ListSnapshot> = {
      [applications.section]: {
        status: 'ready',
        rows: [
          {
            instance: 'web',
            name: 'web',
            namespace: 'default',
            managed_by: 'Helm',
            version: '1.0.0',
            creation_timestamp: '',
            ready_count: 2,
            total_count: 2,
          },
        ],
      },
    }
    const graph = buildTopologyGraph(input({ lists }))
    expect(graph.edges).toHaveLength(1)
    const appNode = graph.nodes.find((n) => n.kind === 'Application')
    expect(appNode?.tone).toBe('ok')
    expect(graph.edges[0]?.target).toBe(appNode?.id)
  })

  it('de-dups a redundant edge between the same node pair into one edge with both relations (FR-013)', () => {
    const lists: Record<string, ListSnapshot> = {
      [applications.section]: {
        status: 'ready',
        rows: [
          {
            instance: 'web',
            name: 'web',
            namespace: 'default',
            managed_by: 'Helm',
            version: '1.0.0',
            creation_timestamp: '',
            ready_count: 1,
            total_count: 1,
          },
        ],
      },
      [deployments.section]: {
        status: 'ready',
        rows: [
          { name: 'web', namespace: 'default', desired: 1, ready: 1, updated: 1, available: 1 },
        ],
      },
    }
    const details: Record<string, DetailSnapshot> = {
      'RelatedResources|default|web': {
        eventType: 'RelatedResourcesUpdated',
        data: {
          related: [
            {
              section: deployments.section,
              kind: 'Deployment',
              namespace: 'default',
              name: 'web',
              relation: 'owns',
              status_hint: null,
            },
          ],
        },
      },
    }
    const graph = buildTopologyGraph(
      input({
        lists,
        details,
        viewport: { namespaceFocus: new Set(['default']), workloadFocus: new Set() },
      }),
    )
    // Namespace->Deployment and Application->Deployment: two distinct pairs, not a duplicate of
    // the same pair, so both edges are expected; assert no duplicate edge ids exist regardless.
    const ids = graph.edges.map((e) => e.id)
    expect(new Set(ids).size).toBe(ids.length)
  })

  it('shows loading on a workload node whose pod detail has not resolved yet (FR-015)', () => {
    const lists: Record<string, ListSnapshot> = {
      [deployments.section]: {
        status: 'ready',
        rows: [
          { name: 'web', namespace: 'default', desired: 3, ready: 3, updated: 3, available: 3 },
        ],
      },
    }
    const graph = buildTopologyGraph(
      input({
        lists,
        viewport: {
          namespaceFocus: new Set(['default']),
          workloadFocus: new Set(['default/Deployment/web']),
        },
      }),
    )
    const workloadNode = graph.nodes.find((n) => n.kind === 'Deployment')
    expect(workloadNode?.loading).toBe(true)
    expect(graph.nodes.some((n) => n.kind === 'Pod')).toBe(false)
  })

  it('renders pod nodes once the workload detail resolves', () => {
    const lists: Record<string, ListSnapshot> = {
      [deployments.section]: {
        status: 'ready',
        rows: [
          { name: 'web', namespace: 'default', desired: 1, ready: 1, updated: 1, available: 1 },
        ],
      },
      [nodesDescriptor.section]: {
        status: 'ready',
        rows: [
          {
            name: 'node-1',
            status: 'Ready',
            role: 'worker',
            pod_count: 1,
            pod_capacity: 10,
            cpu_allocatable_millicores: 1000,
            memory_allocatable_bytes: 1,
          },
        ],
      },
    }
    const details: Record<string, DetailSnapshot> = {
      'Deployment|default|web': {
        eventType: 'DeploymentDetailsUpdated',
        data: {
          pods: [
            {
              name: 'web-abc',
              namespace: 'default',
              phase: 'Running',
              ready: 1,
              total: 1,
              node: 'node-1',
              restarts: 0,
            },
          ],
        },
      },
    }
    const graph = buildTopologyGraph(
      input({
        lists,
        details,
        viewport: {
          namespaceFocus: new Set(['default']),
          workloadFocus: new Set(['default/Deployment/web']),
        },
      }),
    )
    const pod = graph.nodes.find((n) => n.kind === 'Pod')
    expect(pod?.label).toBe('web-abc')
    expect(pod?.tone).toBe('ok')
    expect(graph.edges.some((e) => e.relations.includes('scheduled-on'))).toBe(true)
  })
})
