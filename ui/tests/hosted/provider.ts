/**
 * Provider-specific test plumbing for the live hosted suite (spec 031, T038/T045): the login
 * page of each IdP and its admin API (used to revoke a user). Only the TEST harness knows about
 * providers; the server has no provider code path (FR-020).
 *
 * Node-side HTTPS goes to the provider through the kind node's IP when `FATHOM_HOSTED_IP` is
 * set (the same mapping Chromium gets via --host-resolver-rules), trusting k8s/.tmp/ca.crt.
 */
import { readFileSync } from 'node:fs'
import https from 'node:https'
import { resolve } from 'node:path'
import type { LookupFunction } from 'node:net'

export type Provider = 'rauthy' | 'keycloak'
export const PROVIDER: Provider = process.env.PROVIDER === 'keycloak' ? 'keycloak' : 'rauthy'
export const PASSWORD = 'FathomDemo#2026-Pw1'
const ADMIN_PASSWORD = 'FathomAdmin#2026-Pw1'
const RAUTHY_API_KEY = 'e2e$fathome2eapikeysecretfathome2eapikeysecretfathome2eapikeysecretf'
export const IDP_HOST = PROVIDER === 'rauthy' ? 'auth.localtest.me' : 'keycloak.localtest.me'

const hostedIp = process.env.FATHOM_HOSTED_IP
const lookup: LookupFunction | undefined = hostedIp
  ? (_host, opts, cb) => {
      if (opts.all) cb(null, [{ address: hostedIp, family: 4 }])
      else cb(null, hostedIp, 4)
    }
  : undefined

function caPem(): Buffer | undefined {
  try {
    return readFileSync(resolve(import.meta.dirname, '../../../k8s/.tmp/ca.crt'))
  } catch {
    return undefined
  }
}

interface Reply {
  status: number
  body: string
}

export function idpRequest(
  method: string,
  path: string,
  headers: Record<string, string> = {},
  body?: string,
): Promise<Reply> {
  const ca = caPem()
  return new Promise((ok, fail) => {
    const req = https.request(
      {
        host: IDP_HOST,
        port: 443,
        method,
        path,
        headers: body ? { 'content-length': Buffer.byteLength(body), ...headers } : headers,
        ...(lookup && { lookup }),
        ...(ca ? { ca } : { rejectUnauthorized: false }),
      },
      (res) => {
        let data = ''
        res.setEncoding('utf8')
        res.on('data', (c: string) => (data += c))
        res.on('end', () => ok({ status: res.statusCode ?? 0, body: data }))
      },
    )
    req.on('error', fail)
    if (body) req.write(body)
    req.end()
  })
}

function must(r: Reply, what: string): Reply {
  if (r.status < 200 || r.status >= 300) throw new Error(`${what}: HTTP ${r.status} ${r.body}`)
  return r
}

// ---- Rauthy --------------------------------------------------------------------------------
const rauthyAuth = { authorization: `API-Key ${RAUTHY_API_KEY}` }

async function rauthyUser(username: string): Promise<{ id: string; email: string }> {
  const r = must(await idpRequest('GET', '/auth/v1/users', rauthyAuth), 'rauthy list users')
  const users = JSON.parse(r.body) as { id: string; email: string }[]
  const u = users.find((x) => x.email === `${username}@fathom.test`)
  if (!u) throw new Error(`rauthy user ${username} not found`)
  return u
}

async function rauthySetEnabled(username: string, enabled: boolean): Promise<void> {
  const u = await rauthyUser(username)
  const full = JSON.parse(
    must(await idpRequest('GET', `/auth/v1/users/${u.id}`, rauthyAuth), 'rauthy get user').body,
  ) as Record<string, unknown>
  const body = JSON.stringify({
    email: full.email,
    given_name: full.given_name,
    family_name: full.family_name,
    roles: full.roles,
    groups: full.groups,
    enabled,
    email_verified: full.email_verified,
  })
  must(
    await idpRequest(
      'PUT',
      `/auth/v1/users/${u.id}`,
      { ...rauthyAuth, 'content-type': 'application/json' },
      body,
    ),
    'rauthy update user',
  )
  if (!enabled) {
    // end the user's IdP sessions and refresh tokens (what an admin "revoke" does)
    must(await idpRequest('DELETE', `/auth/v1/sessions/${u.id}`, rauthyAuth), 'rauthy sessions')
  }
}

// ---- Keycloak ------------------------------------------------------------------------------
async function keycloakAdminToken(): Promise<Record<string, string>> {
  const form = new URLSearchParams({
    grant_type: 'password',
    client_id: 'admin-cli',
    username: 'admin',
    password: ADMIN_PASSWORD,
  }).toString()
  const r = must(
    await idpRequest(
      'POST',
      '/realms/master/protocol/openid-connect/token',
      { 'content-type': 'application/x-www-form-urlencoded' },
      form,
    ),
    'keycloak admin token',
  )
  return {
    authorization: `Bearer ${(JSON.parse(r.body) as { access_token: string }).access_token}`,
  }
}

async function keycloakSetEnabled(username: string, enabled: boolean): Promise<void> {
  const auth = await keycloakAdminToken()
  const r = must(
    await idpRequest(
      'GET',
      `/admin/realms/fathom/users?exact=true&username=${encodeURIComponent(username)}`,
      auth,
    ),
    'keycloak find user',
  )
  const [u] = JSON.parse(r.body) as { id: string }[]
  if (!u) throw new Error(`keycloak user ${username} not found`)
  must(
    await idpRequest(
      'PUT',
      `/admin/realms/fathom/users/${u.id}`,
      { ...auth, 'content-type': 'application/json' },
      JSON.stringify({ enabled }),
    ),
    'keycloak update user',
  )
  if (!enabled) {
    must(
      await idpRequest('POST', `/admin/realms/fathom/users/${u.id}/logout`, auth),
      'keycloak logout user',
    )
  }
}

async function keycloakEnsureUser(username: string, group: string): Promise<void> {
  const auth = await keycloakAdminToken()
  const r = await idpRequest(
    'POST',
    '/admin/realms/fathom/users',
    { ...auth, 'content-type': 'application/json' },
    JSON.stringify({
      username,
      enabled: true,
      email: `${username}@fathom.test`,
      emailVerified: true,
      firstName: username,
      lastName: 'Soak',
      groups: [group],
      credentials: [{ type: 'password', value: PASSWORD, temporary: false }],
    }),
  )
  if (r.status !== 201 && r.status !== 409) must(r, 'keycloak create user')
}

async function rauthyEnsureUser(username: string, group: string): Promise<void> {
  const email = `${username}@fathom.test`
  const list = JSON.parse(
    must(await idpRequest('GET', '/auth/v1/users', rauthyAuth), 'rauthy list users').body,
  ) as { id: string; email: string }[]
  let id = list.find((u) => u.email === email)?.id
  const json = { ...rauthyAuth, 'content-type': 'application/json' }
  const base = {
    email,
    given_name: username,
    family_name: 'Soak',
    roles: ['user'],
    groups: [group],
  }
  if (!id) {
    const r = must(
      await idpRequest('POST', '/auth/v1/users', json, JSON.stringify(base)),
      'rauthy create user',
    )
    id = (JSON.parse(r.body) as { id: string }).id
  }
  must(
    await idpRequest(
      'PUT',
      `/auth/v1/users/${id}`,
      json,
      JSON.stringify({ ...base, enabled: true, email_verified: true, password: PASSWORD }),
    ),
    'rauthy set password',
  )
}

/** Create (or reset) a user with the suite password in `group` (soak test). */
export function ensureUser(username: string, group: string): Promise<void> {
  return PROVIDER === 'rauthy'
    ? rauthyEnsureUser(username, group)
    : keycloakEnsureUser(username, group)
}

/** Disable a user and end their provider sessions (refresh tokens stop working). */
export function revokeUser(username: string): Promise<void> {
  return PROVIDER === 'rauthy'
    ? rauthySetEnabled(username, false)
    : keycloakSetEnabled(username, false)
}

/** Re-enable a user disabled by {@link revokeUser}. */
export function restoreUser(username: string): Promise<void> {
  return PROVIDER === 'rauthy'
    ? rauthySetEnabled(username, true)
    : keycloakSetEnabled(username, true)
}
