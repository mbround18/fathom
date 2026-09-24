/**
 * Capability gate (033 US5, FR-011, research R7): `useCapability` asks the backend whether the
 * current identity may do a verb on a resource, and `<Gated>` disables a mutating control (with
 * the reason as its tooltip) unless the answer is a verified "yes". Fails closed: pending,
 * unverifiable, and denied all disable; nothing is ever enabled by default.
 *
 * Backend traffic is lazy and event-driven: one `ResolveCapabilities` per (context, namespace)
 * the first time a control needs it, one `CheckCapability` per distinct question, answers cached
 * here for the 60 s TTL (the backend keeps the same TTL). No idle polling: expiry only marks an
 * answer stale, and a re-ask happens only if a gated control is still mounted.
 */
import { cloneElement, isValidElement, useEffect, type ReactElement, type ReactNode } from 'react'
import { create } from 'zustand'

import type { CapabilityQuery, CapabilityResult, CapabilitySet } from '@/protocol/types'
import { useStore } from '@/store'

export const CAPABILITY_TTL_MS = 60_000

/** A question without its context (always the active one). `namespace: null` = cluster scope. */
export interface CapabilityTarget {
  verb: string
  resource: string
  namespace?: string | null
  apiGroup?: string
  subresource?: string
}

export type CapabilityState =
  | { status: 'pending'; reason: string }
  | { status: 'allowed'; reason?: undefined }
  | { status: 'denied'; reason: string }
  | { status: 'unverified'; reason: string }

interface Entry {
  result: CapabilityResult
  stale: boolean
}

interface CapabilityStore {
  results: Record<string, Entry>
  sets: Record<string, CapabilitySet>
}

export const useCapabilityStore = create<CapabilityStore>(() => ({ results: {}, sets: {} }))

const inflight = new Set<string>()
const resolvedScopes = new Set<string>()
const timers = new Map<string, ReturnType<typeof setTimeout>>()

function keyOf(q: CapabilityQuery): string {
  return JSON.stringify([
    q.context,
    q.namespace ?? null,
    q.verb,
    q.resource,
    q.api_group ?? null,
    q.subresource ?? null,
  ])
}

function scopeKey(context: string, namespace: string | null): string {
  return JSON.stringify([context, namespace])
}

export function toQuery(context: string, t: CapabilityTarget): CapabilityQuery {
  return {
    context,
    namespace: t.namespace ?? null,
    verb: t.verb,
    resource: t.resource,
    api_group: t.apiGroup ?? null,
    subresource: t.subresource ?? null,
  }
}

/** Drop every cached answer (identity/context change, successful RBAC apply). */
export function resetCapabilities(): void {
  for (const t of timers.values()) clearTimeout(t)
  timers.clear()
  inflight.clear()
  resolvedScopes.clear()
  useCapabilityStore.setState({ results: {}, sets: {} })
}

/** `CapabilityChecked`. Answers for a context that is no longer active are discarded (stale). */
export function ingestCapabilityChecked(query: CapabilityQuery, result: CapabilityResult): void {
  const key = keyOf(query)
  inflight.delete(key)
  if (query.context !== useStore.getState().session.activeContext) return
  useCapabilityStore.setState((s) => ({
    results: { ...s.results, [key]: { result, stale: false } },
  }))
  const prev = timers.get(key)
  if (prev !== undefined) clearTimeout(prev)
  timers.set(
    key,
    setTimeout(() => {
      timers.delete(key)
      resolvedScopes.delete(scopeKey(query.context, query.namespace ?? null))
      useCapabilityStore.setState((s) => {
        const entry = s.results[key]
        return entry ? { results: { ...s.results, [key]: { ...entry, stale: true } } } : s
      })
    }, CAPABILITY_TTL_MS),
  )
}

/** `CapabilitiesResolved`: kept for inspection; stale contexts are discarded. */
export function ingestCapabilitySet(set: CapabilitySet): void {
  if (set.context !== useStore.getState().session.activeContext) return
  const key = scopeKey(set.context, set.namespace ?? null)
  useCapabilityStore.setState((s) => ({ sets: { ...s.sets, [key]: set } }))
}

function request(query: CapabilityQuery): void {
  const key = keyOf(query)
  if (inflight.has(key)) return
  inflight.add(key)
  const { send } = useStore.getState()
  const scope = scopeKey(query.context, query.namespace ?? null)
  if (!resolvedScopes.has(scope)) {
    resolvedScopes.add(scope)
    send({
      type: 'ResolveCapabilities',
      data: { context: query.context, namespace: query.namespace ?? null },
    })
  }
  send({ type: 'CheckCapability', data: { query } })
}

const PENDING: CapabilityState = { status: 'pending', reason: 'Checking permissions…' }
const NO_CONTEXT: CapabilityState = {
  status: 'unverified',
  reason: 'Could not verify permissions: not connected to a cluster.',
}

/** Pure mapping from a backend answer to the gate's state. */
export function stateFor(entry: Entry | undefined): CapabilityState {
  if (!entry) return PENDING
  const { result } = entry
  if (!result.verified) {
    return {
      status: 'unverified',
      reason: `Could not verify permissions${result.reason ? `: ${result.reason}` : '.'}`,
    }
  }
  if (result.allowed) return { status: 'allowed' }
  return { status: 'denied', reason: result.reason ?? 'Your identity is not permitted to do this.' }
}

/** Ask whether the current identity may `target`. `null` skips the question (no request). */
export function useCapability(target: CapabilityTarget | null): CapabilityState {
  const context = useStore((s) => s.session.activeContext)
  const query = target && context !== undefined ? toQuery(context, target) : null
  const key = query ? keyOf(query) : ''
  const entry = useCapabilityStore((s) => (key ? s.results[key] : undefined))
  const needs = query !== null && (entry === undefined || entry.stale)
  useEffect(() => {
    if (needs) request(query)
    // `query` is derived from `key`; re-running on the key/need is the intent.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key, needs])
  if (!target) return { status: 'allowed' }
  if (context === undefined) return NO_CONTEXT
  return stateFor(entry)
}

export interface GatedProps extends CapabilityTarget {
  /** The single mutating control (a Button, menu item, ...). Receives `disabled` + `title`. */
  children: ReactElement<Record<string, unknown>>
  /**
   * Hide instead of disable when the identity cannot even `list` the resource (the kind is
   * unlistable, so showing a disabled control would only advertise it).
   */
  hideIfUnlistable?: boolean
}

/**
 * Wrap a mutating control: disabled, with the reason as its tooltip, unless the backend has
 * verified the identity may perform `verb` on `resource`. Every mutating control MUST be wrapped
 * (enforced by tests/unit/gated-coverage.test.ts, SC-003).
 */
export function Gated(props: GatedProps): ReactNode {
  const { children, hideIfUnlistable, ...target } = props
  const cap = useCapability(target)
  const list = useCapability(hideIfUnlistable ? { ...target, verb: 'list' } : null)
  if (!isValidElement(children)) return null
  if (hideIfUnlistable && list.status === 'denied') return null
  if (cap.status === 'allowed') {
    return cloneElement(children, { 'data-capability': 'allowed' })
  }
  return cloneElement(children, {
    disabled: true,
    'aria-disabled': true,
    title: cap.reason,
    'data-capability': cap.status,
    // Belt and braces: a disabled control must not fire even if a wrapper swallows `disabled`.
    onClick: undefined,
    onSelect: undefined,
  })
}

const APPS = new Set(['deployments', 'statefulsets', 'daemonsets', 'replicasets'])
const RBAC = new Set(['roles', 'rolebindings', 'clusterroles', 'clusterrolebindings'])

/** The (resource, apiGroup) a navigator section's objects live under. */
export function capabilityForSection(section: string): { resource: string; apiGroup?: string } {
  if (section === 'helm/releases') return { resource: 'secrets' }
  const resource = section.split('/').pop() ?? section
  if (APPS.has(resource)) return { resource, apiGroup: 'apps' }
  if (RBAC.has(resource)) return { resource, apiGroup: 'rbac.authorization.k8s.io' }
  return { resource }
}

/**
 * Non-hook check for keyboard shortcuts that bypass a `<Gated>` control: true only when the
 * answer is already a verified "yes". A missing answer is requested (for next time) and refused.
 */
export function isAllowedNow(target: CapabilityTarget): boolean {
  const context = useStore.getState().session.activeContext
  if (context === undefined) return false
  const query = toQuery(context, target)
  const entry = useCapabilityStore.getState().results[keyOf(query)]
  if (!entry || entry.stale) request(query)
  return stateFor(entry).status === 'allowed'
}
