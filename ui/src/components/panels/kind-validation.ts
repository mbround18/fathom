/**
 * Client-side mirror of the backend's kind cluster-name rule (030). UX only: the backend
 * re-validates every name before it reaches a subprocess argument.
 */
export function validateKindClusterName(name: string, existing: readonly string[]): string | null {
  if (name.length === 0) return 'Enter a cluster name.'
  if (name.length > 63) return 'Must be at most 63 characters.'
  if (!/^[a-z0-9-]+$/.test(name)) return "Use only lowercase letters, digits, and '-'."
  if (name.startsWith('-') || name.endsWith('-'))
    return 'Must start and end with a letter or digit.'
  if (existing.includes(name)) return `A kind cluster named '${name}' already exists.`
  return null
}

/**
 * 040 US3 (FR-018): the config the bare profile passes to `kind create cluster`, shown before
 * create. Mirrors `BARE_CONFIG` in `src/backend/kind.rs` (a unit test guards drift).
 */
export const KIND_BARE_CONFIG =
  'kind: Cluster\napiVersion: kind.x-k8s.io/v1alpha4\nnodes:\n- role: control-plane\n'
