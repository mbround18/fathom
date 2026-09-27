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
