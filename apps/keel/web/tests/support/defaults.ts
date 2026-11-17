/**
 * Defaults for protocol fields that later specs add to existing wire types, so fixtures written
 * before a field existed spread one constant instead of repeating its value. Data only, so unit
 * tests import it too.
 *
 * Spec 068 (converge): `RunInfo`, `RunHostInfo`, `BindingInfo` and `HostInfo` gained the fields
 * below; the values are what the server sends for a run, host or binding converge never touched.
 */
import type { BindingInfo, HostInfo, RunHostInfo, RunInfo } from '../../src/protocol/wire'

type Added<T, K extends keyof T> = Pick<T, K>

/** A plain (non-converge, non-check) run. */
export const RUN_068: Added<RunInfo, 'kind' | 'check' | 'parent_run'> = {
  kind: 'normal',
  check: false,
  parent_run: null,
}

/** A run host neither covered by a converge nor skipped. */
export const RUN_HOST_068: Added<RunHostInfo, 'covered_by' | 'skip_reason'> = {
  covered_by: null,
  skip_reason: null,
}

/** A binding with no required checks and no runner refusing its manifest (server default timeout). */
export const BINDING_068: Added<BindingInfo, 'required_checks' | 'checks_timeout_secs' | 'runners_refusing_manifest'> = {
  required_checks: [],
  checks_timeout_secs: 3600,
  runners_refusing_manifest: [],
}

/** A host with no open converge and no profile suggestions. */
export const HOST_068: Added<HostInfo, 'converge' | 'suggestions'> = {
  converge: null,
  suggestions: 0,
}

/** A run host row as fixtures before spec 068 wrote it; `runHost` fills the newer fields. */
export type RunHostInput = Omit<RunHostInfo, keyof typeof RUN_HOST_068> & Partial<typeof RUN_HOST_068>

export function runHost(row: RunHostInput): RunHostInfo {
  return { ...RUN_HOST_068, ...row }
}
