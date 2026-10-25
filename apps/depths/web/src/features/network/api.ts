/**
 * The HTTP side of spec 052: known networks, the home/public answer, sessions, and the admin
 * whitelist. Every mutation carries the session's CSRF token (`mutate`); the server checks the role
 * and the network again on each one, so nothing here is the protection.
 */
import { mutate, type FetchLike } from '@/protocol/http'
import type {
  AnswerNetwork,
  NetworkAnswer,
  NetworkView,
  SessionsEnded,
  SessionView,
  TrustDecision,
  WhitelistNetwork,
} from '@/protocol/generated'

export type ApiResult<T> = { ok: true; data: T } | { ok: false; status: number; error: string }

const defaultFetch: FetchLike = (input, init) => globalThis.fetch(input, init)

async function read<T>(res: Promise<Response>): Promise<ApiResult<T>> {
  let r: Response
  try {
    r = await res
  } catch {
    return { ok: false, status: 0, error: 'network' }
  }
  const body: unknown = await r.json().catch(() => null)
  if (r.ok) return { ok: true, data: body as T }
  const error =
    body && typeof body === 'object' && 'error' in body && typeof body.error === 'string' ? body.error : 'error'
  return { ok: false, status: r.status, error }
}

function get<T>(path: string, fetchFn: FetchLike): Promise<ApiResult<T>> {
  return read<T>(fetchFn(path, { credentials: 'same-origin', headers: { Accept: 'application/json' } }))
}

function send<T>(path: string, csrf: string, method: string, body: unknown, fetchFn: FetchLike): Promise<ApiResult<T>> {
  const init: RequestInit = { method, headers: { Accept: 'application/json' } }
  if (body !== undefined) {
    init.headers = { Accept: 'application/json', 'Content-Type': 'application/json' }
    init.body = JSON.stringify(body)
  }
  return read<T>(mutate(path, csrf, init, fetchFn))
}

const user = (id: string): string => `/api/v1/users/${encodeURIComponent(id)}`
/** A CIDR keeps its `/` as a path separator: the server takes the rest of the path as the key. */
const keyPath = (key: string): string => key.split('/').map(encodeURIComponent).join('/')

export const listNetworks = (fetchFn = defaultFetch) => get<NetworkView[]>('/api/v1/networks', fetchFn)

export function answerNetwork(csrf: string, network: string, answer: NetworkAnswer, fetchFn = defaultFetch) {
  const body: AnswerNetwork = { network, answer }
  return send<TrustDecision>('/api/v1/networks/answer', csrf, 'POST', body, fetchFn)
}

export const listSessions = (fetchFn = defaultFetch) => get<SessionView[]>('/api/v1/sessions', fetchFn)

export const endSession = (csrf: string, hint: string, fetchFn = defaultFetch) =>
  send<SessionsEnded>(`/api/v1/sessions/${encodeURIComponent(hint)}`, csrf, 'DELETE', undefined, fetchFn)

export const endOtherSessions = (csrf: string, fetchFn = defaultFetch) =>
  send<SessionsEnded>('/api/v1/sessions/end-others', csrf, 'POST', undefined, fetchFn)

export const userSessions = (id: string, fetchFn = defaultFetch) => get<SessionView[]>(`${user(id)}/sessions`, fetchFn)

export const endUserSessions = (csrf: string, id: string, fetchFn = defaultFetch) =>
  send<SessionsEnded>(`${user(id)}/sessions`, csrf, 'DELETE', undefined, fetchFn)

export const userNetworks = (id: string, fetchFn = defaultFetch) => get<NetworkView[]>(`${user(id)}/networks`, fetchFn)

export function whitelistNetwork(csrf: string, id: string, body: WhitelistNetwork, fetchFn = defaultFetch) {
  return send<NetworkView[]>(`${user(id)}/networks/whitelist`, csrf, 'POST', body, fetchFn)
}

export const removeNetwork = (csrf: string, id: string, key: string, fetchFn = defaultFetch) =>
  send<NetworkView[]>(`${user(id)}/networks/${keyPath(key)}`, csrf, 'DELETE', undefined, fetchFn)
