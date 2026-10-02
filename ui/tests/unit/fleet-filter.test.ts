/**
 * 046-fleet-model US2 (T037): the decision logic behind the fleet view, over a 100-member fleet.
 *
 * `resources/fleet.ts` decides what the operator sees: which members a filter keeps, how many of
 * how many that is (FR-016), which members each group holds (FR-004), which members no group holds
 * (FR-017), and how a rollup's coverage and its gaps read (FR-010, FR-011). Principle III requires
 * a unit test for exactly that class of logic, and SC-007 puts a 100 ms budget on the filter for a
 * fleet of 100.
 */
import { describe, expect, it } from 'vitest'

import type { FleetGroup, FleetMember } from '@/protocol/types'
import {
  coverageLabel,
  coverageState,
  filterMembers,
  gapDetail,
  gapHeadline,
  isGrouped,
  matchesSelector,
  membersOfGroup,
  orderedGaps,
  partitionFleet,
  selectorReadable,
  showingLabel,
} from '@/resources/fleet'

const REGIONS = ['us-east-1', 'us-west-2', 'eu-west-1', 'ap-south-1']
const OWNERS = ['payments', 'search', 'platform']

/** Round-robin pick, so the fixture's labels are total functions of the index. */
function pick(values: readonly string[], i: number): string {
  return values[i % values.length] ?? values[0] ?? ''
}

/** A hundred contexts shaped like the ones a real kubeconfig holds — ARNs and GKE strings. */
function fleet(n = 100): FleetMember[] {
  return Array.from({ length: n }, (_, i) => {
    const env = i % 5 === 0 ? 'prod' : i % 5 === 1 ? 'staging' : 'dev'
    const member: FleetMember = {
      context:
        i % 2 === 0
          ? `arn:aws:eks:${pick(REGIONS, i)}:1234567890${i}:cluster/cluster-${i}`
          : `gke_my-project_${pick(REGIONS, i)}_cluster-${i}`,
      display_name: i % 3 === 0 ? `cluster ${i}` : null,
      labels: { env, region: pick(REGIONS, i), owner: pick(OWNERS, i) },
      origins: {},
      availability: i === 97 ? 'Absent' : 'Present',
    }
    // Four members carry no labels at all: they belong to no group (FR-017).
    return i % 25 === 24 ? { ...member, labels: {} } : member
  })
}

const group = (name: string, selector: string): FleetGroup => ({ name, selector, origin: 'local' })

const MEMBERS = fleet()
const PROD = group('prod', 'env=prod')
const US = group('us', 'region in (us-east-1, us-west-2)')
const OWNED = group('owned', 'owner')
const NONE = group('staging-eu', 'env=staging, region=eu-central-1')

describe('membership follows the labels, with nothing filed per cluster', () => {
  it('holds every member a selector matches, and only those', () => {
    const prod = membersOfGroup(MEMBERS, PROD)
    expect(prod).toHaveLength(MEMBERS.filter((m) => m.labels.env === 'prod').length)
    expect(prod.every((m) => m.labels.env === 'prod')).toBe(true)
  })

  it('puts one member under every group it matches, counted once in each', () => {
    const both = MEMBERS.filter(
      (m) => m.labels.env === 'prod' && m.labels.region === 'us-east-1',
    ).map((m) => m.context)
    expect(both.length).toBeGreaterThan(0)
    for (const context of both) {
      expect(membersOfGroup(MEMBERS, PROD).filter((m) => m.context === context)).toHaveLength(1)
      expect(membersOfGroup(MEMBERS, US).filter((m) => m.context === context)).toHaveLength(1)
    }
  })

  it('keeps a group that matches nothing, as a group that matches nothing (FR-005)', () => {
    expect(membersOfGroup(MEMBERS, NONE)).toEqual([])
    const view = partitionFleet(MEMBERS, [PROD, NONE])
    expect(view.groups.map((g) => g.group.name)).toEqual(['prod', 'staging-eu'])
    expect(view.groups[1]?.members).toEqual([])
  })

  it('separates a selector that matches nothing from one that cannot be read at all', () => {
    const broken = group('broken', 'env = = prod')
    expect(selectorReadable('env in (prod, staging)')).toBe(true)
    expect(selectorReadable('!owner')).toBe(true)
    expect(selectorReadable('env = = prod')).toBe(false)

    const view = partitionFleet(MEMBERS, [NONE, broken])
    // Both hold no members, and the view can still tell the operator which is which: one matched
    // nothing, the other was never resolved (FR-005, FR-011).
    expect(view.groups[0]?.members).toEqual([])
    expect(view.groups[0]?.readable).toBe(true)
    expect(view.groups[1]?.members).toEqual([])
    expect(view.groups[1]?.readable).toBe(false)
  })

  it('reads the kubectl -l subset the backend accepts, and nothing it does not', () => {
    const labels = { env: 'prod', region: 'us-east-1' }
    expect(matchesSelector('env=prod', labels)).toBe(true)
    expect(matchesSelector('env==prod', labels)).toBe(true)
    expect(matchesSelector('env!=dev', labels)).toBe(true)
    expect(matchesSelector('env!=prod', labels)).toBe(false)
    expect(matchesSelector('env in (prod, staging)', labels)).toBe(true)
    expect(matchesSelector('region notin (eu-west-1)', labels)).toBe(true)
    expect(matchesSelector('env, region', labels)).toBe(true)
    expect(matchesSelector('!owner', labels)).toBe(true)
    expect(matchesSelector('!env', labels)).toBe(false)
    expect(matchesSelector('env=prod, region=us-west-2', labels)).toBe(false)
    // An expression this module cannot read matches nothing: the group renders as matching
    // nothing rather than the UI inventing a membership the backend never resolved.
    expect(matchesSelector('env = = prod', labels)).toBe(false)
  })
})

describe('an unlabelled member is reachable, not lost (FR-017)', () => {
  it('lists every member no group matched', () => {
    const view = partitionFleet(MEMBERS, [PROD, US, OWNED])
    const unlabelled = MEMBERS.filter((m) => Object.keys(m.labels).length === 0)
    expect(unlabelled.length).toBe(4)
    expect(view.ungrouped.map((m) => m.context)).toEqual(unlabelled.map((m) => m.context))
    for (const m of unlabelled) expect(isGrouped(m, [PROD, US, OWNED])).toBe(false)
  })

  it('counts an Absent member like any other: it keeps its metadata and its groups (FR-009)', () => {
    const absent = MEMBERS.find((m) => m.availability === 'Absent')
    expect(absent).toBeDefined()
    const view = partitionFleet(MEMBERS, [PROD, US, OWNED])
    const listed = view.groups.flatMap((g) => g.members).concat(view.ungrouped)
    expect(listed.map((m) => m.context)).toContain(absent?.context)
  })
})

describe('a filter states how many of how many (FR-016)', () => {
  it('counts the shown against the whole fleet, not against itself', () => {
    const view = partitionFleet(MEMBERS, [PROD], 'env=prod')
    const prod = MEMBERS.filter((m) => m.labels.env === 'prod').length
    expect(view.shown).toBe(prod)
    expect(view.total).toBe(100)
    expect(showingLabel(view.shown, view.total)).toBe(`Showing ${prod} of 100 clusters`)
    expect(showingLabel(1, 1)).toBe('Showing 1 of 1 cluster')
  })

  it('matches on context name, display name and key=value labels', () => {
    expect(filterMembers(MEMBERS, 'gke_').every((m) => m.context.startsWith('gke_'))).toBe(true)
    expect(filterMembers(MEMBERS, 'owner=payments').length).toBeGreaterThan(0)
    expect(
      filterMembers(MEMBERS, 'owner=payments').every((m) => m.labels.owner === 'payments'),
    ).toBe(true)
    expect(filterMembers(MEMBERS, '')).toHaveLength(100)
    expect(filterMembers(MEMBERS, 'no-such-cluster')).toHaveLength(0)
  })

  it('filters a 100-member fleet inside the 100 ms input budget (SC-007)', () => {
    const queries = ['e', 'en', 'env', 'env=', 'env=p', 'env=pr', 'env=pro', 'env=prod']
    const groups = [PROD, US, OWNED, NONE]
    // Warm the JIT so the measurement is of steady-state filtering, not of first compilation.
    for (const q of queries) partitionFleet(MEMBERS, groups, q)

    let slowest = 0
    for (const q of queries) {
      const started = performance.now()
      const view = partitionFleet(MEMBERS, groups, q)
      slowest = Math.max(slowest, performance.now() - started)
      expect(view.total).toBe(100)
    }
    expect(slowest).toBeLessThan(100)
  })
})

describe('a rollup never states a sum without its coverage (FR-010, FR-011)', () => {
  it('reads coverage as n of m, and calls a partial read partial', () => {
    expect(coverageLabel(1, 4)).toBe('1 of 4 clusters')
    expect(coverageLabel(1, 1)).toBe('1 of 1 cluster')
    expect(coverageState(1, 4)).toBe('partial')
    expect(coverageState(4, 4)).toBe('complete')
  })

  it('keeps the four gap reasons four, and quotes the detail it was given', () => {
    const headlines = [
      gapHeadline({ kind: 'notPermitted', detail: 'nodes is forbidden' }),
      gapHeadline({ kind: 'unreachable', detail: 'connect timed out' }),
      gapHeadline({ kind: 'absent' }),
      gapHeadline({ kind: 'pending' }),
    ]
    expect(new Set(headlines).size).toBe(4)
    expect(headlines[0]).toMatch(/not permitted/i)
    expect(gapDetail({ kind: 'notPermitted', detail: 'nodes is forbidden' })).toBe(
      'nodes is forbidden',
    )
    expect(gapDetail({ kind: 'absent' })).toBeNull()
    expect(gapDetail({ kind: 'pending' })).toBeNull()
  })

  it('orders gaps by context so one rollup renders the same way twice', () => {
    const gaps = [
      { context: 'zeta', reason: { kind: 'absent' } as const },
      { context: 'alpha', reason: { kind: 'pending' } as const },
    ]
    expect(orderedGaps(gaps).map((g) => g.context)).toEqual(['alpha', 'zeta'])
    // Pure: the input is not reordered in place.
    expect(gaps.map((g) => g.context)).toEqual(['zeta', 'alpha'])
  })
})
