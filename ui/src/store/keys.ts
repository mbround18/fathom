/**
 * Cache keys for the store's `lists` and `details` maps (data-model.md §3).
 *
 * `Route.context` and `Route.namespace` are nullable on the wire: a null context (global
 * routes such as cluster events) is keyed as `-`, and a null namespace ("all namespaces") as `*`.
 */
import type { Route } from '@/protocol/types'

export type ListKey = string
export type DetailKey = string

export const NO_CONTEXT = '-'
export const ALL_NAMESPACES = '*'

export function listKey(
  context: string | null | undefined,
  section: string,
  scope: Route['scope'],
  ns: string | null | undefined,
): ListKey {
  return `${context ?? NO_CONTEXT}|${section}|${scope}|${ns ?? ALL_NAMESPACES}`
}

/** 031: key of a section in the store's `denied` map. */
export function deniedKey(context: string | null | undefined, section: string): string {
  return `${context ?? NO_CONTEXT}|${section}`
}

export function listKeyOf(route: Route): ListKey {
  return listKey(route.context, route.section, route.scope, route.namespace)
}

export function detailKey(
  context: string | null | undefined,
  kind: string,
  ns: string | null | undefined,
  name: string,
): DetailKey {
  return `${context ?? NO_CONTEXT}|${kind}|${ns ?? ALL_NAMESPACES}|${name}`
}

/** The section a Flux origin click-through (spec 005) opens its owner in. */
export const FLUX_RESOURCE_SECTION = 'integrations/flux-resource'

/**
 * The `kind` segment a target's detail is cached under.
 *
 * A Flux origin is keyed by the real k8s kind it carries (`HelmRelease`, `Kustomization`) so the
 * two origin kinds cannot share one entry — but that kind string is also a resource kind in its
 * own right, and a Flux-managed release whose `HelmRelease` CR sits in its own namespace under
 * its own name (the default: `src/backend/helm.rs::origin_from_labels` falls back to the release's
 * namespace when the Flux label is absent) then keys identically to the release itself. At wide
 * both panes are on screen at once (044), so whichever read landed last blanked the other: the
 * loser's `selectDetail` rejects the winner's `eventType` and every detail-only section falls back
 * to its skeleton for good. Namespacing the origin's kind keeps both entries.
 */
export function detailKind(section: string, kind: string): string {
  return section === FLUX_RESOURCE_SECTION ? `${FLUX_RESOURCE_SECTION}|${kind}` : kind
}

/** `detailKey` for an inspector target, applying `detailKind`'s Flux-origin namespacing. */
export function detailKeyOf(
  context: string | null | undefined,
  target: { section: string; kind: string; namespace: string | null; name: string },
): DetailKey {
  return detailKey(context, detailKind(target.section, target.kind), target.namespace, target.name)
}
