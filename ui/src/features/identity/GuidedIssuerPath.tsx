/**
 * 045: the guided OIDC path. Turns an issuer URL into a verified identity configuration, showing
 * the real artifact at every step — the discovery document, the decoded claims, the API server's
 * own flags — and writing nothing until the operator saves (FR-007).
 *
 * A `Sheet`, not a `Dialog`: `IssuerSettings` is mounted *inside* an open `DialogContent` at
 * `IdentityChip.tsx:49`, so a dialog here would nest one inside another. A sheet layers over it,
 * which is what lets the same component open unchanged from all three surfaces (FR-029).
 *
 * The body is guarded by `{open && …}` (042's `RoleAssignmentLauncher:47`), so the draft is built
 * fresh on every open and discarded on close — FR-007 by construction, not by cleanup.
 */
import { useEffect, useReducer, useState } from 'react'
import { cn } from 'cn'

import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetFooter,
  SheetHeader,
  SheetTitle,
} from '@/components/ui/sheet'
import type {
  ApiServerAuthComparison,
  ApiServerAuthRead,
  DraftSignInResult,
  FathomRoleMap,
  IssuerOrigin,
  OidcIssuerConfig,
  OidcSignInMethod,
} from '@/protocol/types'
import { OIDC_DRAFT_SESSION_KEY } from '@/protocol/types'
import { buildAuthenticationConfiguration, prefillFromIssuer } from '@/resources/oidcAuthConfig'
import { buildKubeadmFlags } from '@/resources/oidcGenerator'
import { useStore } from '@/store'
import { waitForCommandResult } from '@/store/actions'

import {
  canAdvance,
  EMPTY_ISSUER_CONFIG,
  EMPTY_ROLE_MAP,
  guidedReducer,
  initialGuidedDraft,
  isFrozen,
  issuerMismatch,
  stepsOf,
  type GuidedAction,
  type GuidedDraft,
  type GuidedStep,
} from './guided-state'

/** Step titles, in the operator's words rather than the state machine's. */
const STEP_TITLES: Record<GuidedStep, string> = {
  'issuer-url': 'Issuer URL',
  discovery: 'What the provider advertises',
  claims: 'Claims and prefixes',
  'test-sign-in': 'Test sign-in',
  'cluster-compare': 'What this cluster accepts',
  'fathom-roles': 'Fathom role groups',
  'review-save': 'Review and save',
  bindings: 'Cluster bindings',
  'apiserver-write': 'API server configuration',
}

/**
 * The key a draft sign-in's progress and errors arrive under: `DRAFT_SESSION_KEY` in
 * `src/backend/oidc/session.rs`, mirrored here because it is a Rust constant with no generated TS.
 *
 * `OidcSignInProgress` and `OidcError` are keyed by issuer id, and the draft deliberately does
 * *not* reuse a saved one: `sign_in` cancels any in-flight sign-in for that id, so a draft that
 * borrowed a saved id would kill the operator's live session mid-edit. A saved id is a lowercase
 * slug, so the colon makes a collision unrepresentable.
 */
export const DRAFT_SIGN_IN_KEY = OIDC_DRAFT_SESSION_KEY

export interface GuidedIssuerPathProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  /** Pre-filled for an existing issuer's "Review…"; absent for "Add issuer…" (FR-016). */
  issuer?: OidcIssuerConfig
  /** `SeededFromEnvironment` opens the path read-only and names where it is changed (FR-032). */
  origin?: IssuerOrigin
}

export function GuidedIssuerPath({ open, onOpenChange, issuer, origin }: GuidedIssuerPathProps) {
  const roleMap = useStore((s) => s.identity.config.role_map)
  return (
    <Sheet open={open} onOpenChange={onOpenChange}>
      <SheetContent
        side="right"
        className="w-[calc(100vw-2rem)] sm:max-w-xl"
        data-testid="guided-issuer-path"
      >
        <SheetHeader>
          <SheetTitle>{issuer ? `Review ${issuer.id}` : 'Add issuer'}</SheetTitle>
          <SheetDescription>
            Nothing is written until you save. Closing this leaves your configuration unchanged.
          </SheetDescription>
        </SheetHeader>
        {/* `initialIdentityState` seeds `role_map` as an empty object until `OidcConfigLoaded`
            arrives, so the three slots are defaulted rather than read as present. */}
        {open && (
          <Body
            issuer={issuer ?? EMPTY_ISSUER_CONFIG}
            roleMap={{ ...EMPTY_ROLE_MAP, ...roleMap }}
            origin={origin ?? 'Saved'}
          />
        )}
      </SheetContent>
    </Sheet>
  )
}

/** The last probe, so FR-003's verbatim failure can name the URL it was tried against. */
type ProbeState =
  | { state: 'idle' }
  | { state: 'running'; url: string }
  | { state: 'failed'; url: string; error: string }

function Body({
  issuer,
  roleMap,
  origin,
}: {
  issuer: OidcIssuerConfig
  roleMap: FathomRoleMap
  origin: IssuerOrigin
}) {
  const [draft, dispatch] = useReducer(guidedReducer, undefined, () =>
    initialGuidedDraft(issuer, roleMap, origin),
  )
  const steps = stepsOf(draft)
  const index = steps.indexOf(draft.step)

  const send = useStore((s) => s.send)
  const request = useStore((s) => s.request)
  const activeContext = useStore((s) => s.session.activeContext)
  const storeDiscovery = useStore((s) => s.identity.draftDiscovery)
  const storeSignIn = useStore((s) => s.identity.draftSignIn)
  const storeComparison = useStore((s) =>
    activeContext === undefined ? undefined : s.identity.apiServerAuth[activeContext],
  )
  const [probe, setProbe] = useState<ProbeState>({ state: 'idle' })

  // The three replies the path renders arrive in the identity slice, which is the single source
  // of truth for them; the reducer copies them into the draft so a step renders one coherent
  // object. Each is dropped by the reducer when it no longer belongs to this draft.
  useEffect(() => {
    if (storeDiscovery) {
      dispatch({
        type: 'discovered',
        issuerUrl: storeDiscovery.issuer_url,
        discovery: storeDiscovery.discovery,
      })
    }
  }, [storeDiscovery])
  useEffect(() => {
    if (storeSignIn) dispatch({ type: 'signed-in', result: storeSignIn })
  }, [storeSignIn])
  useEffect(() => {
    if (storeComparison) dispatch({ type: 'compared', comparison: storeComparison })
  }, [storeComparison])

  const probeUrl = draft.config.issuer_url.trim()
  const runProbe = () => {
    if (probeUrl === '' || probe.state === 'running') return
    setProbe({ state: 'running', url: probeUrl })
    void (async () => {
      try {
        const id = await request({
          type: 'OidcDiscoverDraft',
          data: { issuer_url: probeUrl, ca_bundle_path: draft.config.ca_bundle_path },
        })
        const result = await waitForCommandResult(id)
        if (result === 'timeout') {
          setProbe({ state: 'failed', url: probeUrl, error: 'The provider did not answer.' })
        } else if (!result.ok) {
          // Verbatim: the transport or HTTP error the backend saw, not a Fathom paraphrase.
          setProbe({
            state: 'failed',
            url: probeUrl,
            error: result.error ?? 'The discovery document could not be fetched.',
          })
        } else {
          setProbe({ state: 'idle' })
        }
      } catch (err) {
        setProbe({
          state: 'failed',
          url: probeUrl,
          error: err instanceof Error ? err.message : String(err),
        })
      }
    })()
  }

  // T039/FR-026: the comparison belongs to a named context. Entering the step reads it, and so
  // does a context change underneath the open path; until the new read lands the old one is
  // rendered with the context it came from and marked stale.
  useEffect(() => {
    if (draft.step !== 'cluster-compare' || activeContext === undefined) return
    if (storeComparison?.read.context === activeContext) return
    send({ type: 'ReadApiServerAuth' })
  }, [draft.step, activeContext, storeComparison, send])

  return (
    <div
      className="flex min-h-0 flex-1 flex-col gap-3 overflow-y-auto px-4"
      data-testid="guided-body"
    >
      <ol className="flex flex-wrap gap-1 text-xs" data-testid="guided-steps">
        {steps.map((step, i) => (
          <li key={step}>
            <Button
              size="xs"
              variant={step === draft.step ? 'default' : 'ghost'}
              data-testid={`guided-step-${step}`}
              aria-current={step === draft.step ? 'step' : undefined}
              // Only backwards is unconditional (FR-025); forwards obeys the same refusals `next`
              // does, so a blocking step cannot be skipped past.
              disabled={i > index && !canAdvance(draft)}
              onClick={() => dispatch({ type: 'goto', step })}
            >
              <span className={cn(step === draft.step && 'font-semibold')}>{i + 1}</span>
              {STEP_TITLES[step]}
            </Button>
          </li>
        ))}
      </ol>

      {/*
        One section per step. Still unfilled: T042 (role groups), T050 (summary), T044 (bindings),
        T062 (API server write).
      */}
      <section
        className="flex min-h-0 flex-1 flex-col gap-3 text-xs"
        aria-label={STEP_TITLES[draft.step]}
        data-testid={`guided-step-body-${draft.step}`}
      >
        <StepBody
          draft={draft}
          dispatch={dispatch}
          probe={probe}
          runProbe={runProbe}
          activeContext={activeContext}
          onTestSignIn={(method) =>
            send({ type: 'OidcTestSignIn', data: { draft: { config: draft.config, method } } })
          }
        />
      </section>

      <SheetFooter className="flex-row justify-between gap-2 px-0">
        <Button
          size="sm"
          variant="outline"
          data-testid="guided-back"
          disabled={index <= 0}
          onClick={() => dispatch({ type: 'back' })}
        >
          Back
        </Button>
        <Button
          size="sm"
          data-testid="guided-next"
          disabled={index >= steps.length - 1 || !canAdvance(draft)}
          onClick={() => dispatch({ type: 'next' })}
        >
          Next
        </Button>
      </SheetFooter>
    </div>
  )
}

interface StepProps {
  draft: GuidedDraft
  dispatch: (action: GuidedAction) => void
  probe: ProbeState
  runProbe: () => void
  activeContext: string | undefined
  onTestSignIn: (method: OidcSignInMethod) => void
}

function StepBody(p: StepProps) {
  switch (p.draft.step) {
    case 'issuer-url':
      return <IssuerUrlStep {...p} />
    case 'discovery':
      return <DiscoveryStep {...p} />
    case 'test-sign-in':
      return <TestSignInStep {...p} />
    case 'claims':
      return <ClaimsStep {...p} />
    case 'cluster-compare':
      return <ClusterCompareStep {...p} />
    default:
      return null
  }
}

/** One labelled text field, the density `IssuerSettings` already uses. */
function Field({
  label,
  testId,
  value,
  onChange,
  placeholder,
  hint,
  disabled,
  onBlur,
}: {
  label: string
  testId: string
  value: string
  onChange: (v: string) => void
  placeholder?: string
  hint?: string
  disabled?: boolean
  onBlur?: () => void
}) {
  return (
    <label className="flex flex-col gap-1 text-xs">
      <span className="text-muted-foreground">{label}</span>
      <Input
        data-testid={testId}
        value={value}
        placeholder={placeholder}
        disabled={disabled}
        onChange={(e) => onChange(e.target.value)}
        onBlur={onBlur}
      />
      {hint && <span className="text-muted-foreground">{hint}</span>}
    </label>
  )
}

/** A `field: value` line showing the real document's own words, never a summary of them. */
function Row({ label, value, testId }: { label: string; value: string; testId?: string }) {
  return (
    <div className="flex flex-wrap items-baseline gap-2">
      <span className="text-muted-foreground">{label}</span>
      <span className="min-w-0 font-mono break-all" data-testid={testId}>
        {value}
      </span>
    </div>
  )
}

/**
 * T023, step 1. The four strings the probe needs, and the probe itself: leaving the URL field
 * fetches the document (US1 #1), and a failure can be retried here with a CA bundle supplied
 * without losing the rest of the draft (FR-003).
 */
function IssuerUrlStep({ draft, dispatch, probe, runProbe }: StepProps) {
  const frozen = isFrozen(draft)
  const set = (patch: Partial<OidcIssuerConfig>) => dispatch({ type: 'edit-config', patch })
  const url = draft.config.issuer_url.trim()

  return (
    <>
      <p className="text-muted-foreground">
        Paste the provider&apos;s issuer URL. Fathom fetches its discovery document and shows you
        what the provider itself declares, before anything is saved.
      </p>
      <div className="grid grid-cols-1 gap-2 sm:grid-cols-2">
        <Field
          label="Issuer URL"
          testId="guided-issuer-url"
          value={draft.config.issuer_url}
          placeholder="https://idp.example.com/realms/fathom"
          disabled={frozen}
          onChange={(issuer_url) => set({ issuer_url })}
          // US1 #1: "when the operator leaves the URL field". A URL already probed is left alone.
          onBlur={() => {
            if (url !== '' && draft.discovery === undefined && probe.state !== 'running') runProbe()
          }}
        />
        <Field
          label="Issuer id"
          testId="guided-issuer-id"
          value={draft.config.id}
          hint="Keys the stored credentials."
          disabled={frozen}
          onChange={(id) => set({ id })}
        />
        <Field
          label="Client ID (public client)"
          testId="guided-client-id"
          value={draft.config.client_id}
          disabled={frozen}
          onChange={(client_id) => set({ client_id })}
        />
        <Field
          label="Private CA bundle path (optional)"
          testId="guided-ca-bundle"
          value={draft.config.ca_bundle_path ?? ''}
          hint="For a provider behind a private CA."
          disabled={frozen}
          onChange={(v) => set({ ca_bundle_path: v.trim() === '' ? null : v })}
        />
      </div>
      <div className="flex flex-wrap items-center gap-2">
        <Button
          size="sm"
          variant="outline"
          data-testid="guided-probe"
          disabled={url === '' || probe.state === 'running'}
          onClick={runProbe}
        >
          {probe.state === 'failed' ? 'Retry' : 'Fetch discovery document'}
        </Button>
        {probe.state === 'running' && (
          <span className="text-muted-foreground" data-testid="guided-probe-running">
            Fetching {probe.url}…
          </span>
        )}
        {draft.discovery && probe.state !== 'failed' && (
          <span className="text-muted-foreground" data-testid="guided-probe-done">
            Fetched. Continue to see what it declares.
          </span>
        )}
      </div>
      {probe.state === 'failed' && (
        <p role="alert" className="text-status-error" data-testid="guided-probe-error">
          {probe.error} (tried <span className="font-mono break-all">{probe.url}</span>)
        </p>
      )}
    </>
  )
}

/**
 * T023, step 2. The discovery document as the provider wrote it — endpoints, signing keys, and
 * the scopes and claims it advertises (FR-001) — and the one refusal in the path: a declared
 * `issuer` that disagrees with the URL typed (FR-002).
 */
function DiscoveryStep({ draft, probe, runProbe }: StepProps) {
  const d = draft.discovery
  const mismatch = issuerMismatch(draft)
  if (!d) {
    return (
      <>
        <p className="text-muted-foreground" data-testid="guided-discovery-empty">
          Nothing fetched yet. Go back a step and fetch the discovery document.
        </p>
        <div>
          <Button
            size="sm"
            variant="outline"
            data-testid="guided-probe"
            disabled={probe.state === 'running'}
            onClick={runProbe}
          >
            Fetch discovery document
          </Button>
        </div>
      </>
    )
  }

  return (
    <>
      {mismatch && (
        <div
          role="alert"
          className="border-status-error border p-2"
          data-testid="guided-issuer-mismatch"
        >
          <p className="text-status-error">
            This provider declares a different issuer than the URL you entered. Tokens carry the
            declared value in <span className="font-mono">iss</span>, so every token from it would
            fail validation. Correct the URL before continuing.
          </p>
          <Row label="You entered" value={mismatch.typed} testId="guided-mismatch-typed" />
          <Row label="It declares" value={mismatch.declared} testId="guided-mismatch-declared" />
        </div>
      )}
      <div className="flex flex-col gap-1" data-testid="guided-discovery">
        <Row label="Declared issuer" value={d.issuer} testId="guided-declared-issuer" />
        <Row
          label="Authorization endpoint"
          value={d.authorization_endpoint}
          testId="guided-authorization-endpoint"
        />
        <Row label="Token endpoint" value={d.token_endpoint} testId="guided-token-endpoint" />
        <Row label="Signing keys (JWKS)" value={d.jwks_uri} testId="guided-jwks-uri" />
        <Row
          label="Device-code endpoint"
          value={d.device_authorization_endpoint ?? 'not advertised'}
          testId="guided-device-endpoint"
        />
        <Row
          label="End-session endpoint"
          value={d.end_session_endpoint ?? 'not advertised'}
          testId="guided-end-session-endpoint"
        />
      </div>
      <p data-testid="guided-device-code-support">
        {d.device_authorization_endpoint
          ? 'This provider supports the device-code flow, so the test sign-in can run without a browser.'
          : 'This provider does not advertise the device-code flow, so the test sign-in needs a browser.'}
      </p>
      <Advertised
        label="Scopes the provider advertises"
        testId="guided-scopes-supported"
        values={d.scopes_supported}
      />
      <Advertised
        label="Claims the provider advertises"
        testId="guided-claims-supported"
        values={d.claims_supported}
      />
    </>
  )
}

/** A provider may omit either list; that means "not advertised", not "none" (`Discovery`'s doc). */
function Advertised({
  label,
  testId,
  values,
}: {
  label: string
  testId: string
  values: readonly string[]
}) {
  return (
    <div className="flex flex-col gap-1">
      <span className="text-muted-foreground">{label}</span>
      {values.length === 0 ? (
        <span data-testid={`${testId}-none`}>
          The document does not advertise this list, which means it says nothing about it — not that
          there are none.
        </span>
      ) : (
        <ul className="flex flex-wrap gap-1" data-testid={testId}>
          {values.map((v) => (
            <li
              key={v}
              className="border border-border px-1 font-mono"
              data-testid={`${testId}-${v}`}
            >
              {v}
            </li>
          ))}
        </ul>
      )}
    </div>
  )
}

/**
 * The test sign-in. tasks.md names no UI task for this step — T027–T029 are its backend half and
 * T030 starts at the claims — but FR-004 and US2 #1/#5 are UI behaviour, and the claims step has
 * nothing to show without it, so the controls live here.
 *
 * It writes nothing: the draft's tokens stay in the backend's in-memory `Live` slot and never
 * reach the keyring (contracts/protocol.md, `OidcTestSignIn`).
 */
function TestSignInStep({ draft, onTestSignIn }: StepProps) {
  const progress = useStore((s) => s.identity.progress[DRAFT_SIGN_IN_KEY])
  const error = useStore((s) => s.identity.errors[DRAFT_SIGN_IN_KEY])
  const deviceCode = draft.discovery?.device_authorization_endpoint
  const result = draft.signIn

  return (
    <>
      <p className="text-muted-foreground">
        A real sign-in against this draft. It proves the client id and shows the claims a token
        actually carries — and persists nothing: no issuer, no credential.
      </p>
      {progress?.type === 'DeviceCode' ? (
        <div className="border border-border p-2" data-testid="guided-device-prompt">
          <p>
            Open <span className="font-mono">{progress.data.verification_uri}</span> and enter the
            code:
          </p>
          <p className="font-mono text-base tracking-widest" data-testid="guided-device-code">
            {progress.data.user_code}
          </p>
        </div>
      ) : (
        progress && (
          <p data-testid="guided-sign-in-progress" data-stage={progress.type}>
            {progress.type === 'Awaiting'
              ? 'Waiting for the browser sign-in to finish…'
              : 'Validating the ID token…'}
          </p>
        )
      )}
      {error && (
        <p role="alert" className="text-status-error" data-testid="guided-sign-in-error">
          {error.stage}: {error.message}
        </p>
      )}
      {result && (
        <div className="flex flex-col gap-1" data-testid="guided-sign-in-result">
          <Row label="Signed in as" value={result.username} testId="guided-sign-in-username" />
          <Row
            label="Groups the token carried"
            value={result.groups.length === 0 ? 'none' : result.groups.join(', ')}
            testId="guided-sign-in-groups"
          />
          <p className="text-muted-foreground" data-testid="guided-sign-in-recorded">
            A record of this one sign-in, not a live session: nothing was written to the keyring and
            no token is refreshed. What it proved holds until{' '}
            {new Date(result.expires_at * 1000).toISOString()}.
          </p>
        </div>
      )}
      <div className="flex flex-wrap gap-2">
        <Button
          size="sm"
          data-testid="guided-test-sign-in"
          disabled={progress !== undefined}
          onClick={() => onTestSignIn('Browser')}
        >
          {result ? 'Test again with browser' : 'Run test sign-in'}
        </Button>
        <Button
          size="sm"
          variant="outline"
          data-testid="guided-test-sign-in-device"
          disabled={progress !== undefined || !deviceCode}
          title={deviceCode ? undefined : 'This provider does not advertise the device-code flow.'}
          onClick={() => onTestSignIn('DeviceCode')}
        >
          Use device code
        </Button>
      </div>
      <p className="text-muted-foreground">
        You can skip this and save untested — the summary then records what was not verified.
      </p>
    </>
  )
}

/**
 * T030 and T031. The claims step comes *after* the sign-in because FR-005 picks the username and
 * groups claims from the claims a real token carried. With the sign-in skipped (FR-015) it falls
 * back to the claims the discovery document advertises, and says so on screen.
 *
 * The prefixes live here too (T031/FR-011): they are the fields that decide whether a binding
 * matches, and the preview below is the only place their effect is visible before a save.
 */
function ClaimsStep({ draft, dispatch }: StepProps) {
  const frozen = isFrozen(draft)
  const set = (patch: Partial<OidcIssuerConfig>) => dispatch({ type: 'edit-config', patch })
  const observed = draft.signIn
  const names = observed
    ? observed.claims.map((c) => c.name)
    : (draft.discovery?.claims_supported ?? [])
  const { username_claim, groups_claim, username_prefix, groups_prefix } = draft.config

  return (
    <>
      {observed ? (
        <p className="text-muted-foreground" data-testid="guided-claims-observed">
          These are the claims the ID token from your test sign-in actually carried.
        </p>
      ) : (
        <p className="text-muted-foreground" data-testid="guided-claims-advertised">
          No test sign-in was run, so these are the claims the provider&apos;s discovery document
          advertises — what it says it can emit, not what a token was observed to carry.
        </p>
      )}

      {observed && (
        <table className="w-full border border-border" data-testid="guided-claims-table">
          <thead>
            <tr className="text-muted-foreground">
              <th className="px-1 text-left font-normal">Claim</th>
              <th className="px-1 text-left font-normal">Value</th>
            </tr>
          </thead>
          <tbody>
            {observed.claims.map((c) => (
              <tr key={c.name} data-testid={`guided-claim-${c.name}`}>
                <td className="px-1 font-mono">{c.name}</td>
                <td className="px-1 font-mono break-all">{c.value}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
      {observed && observed.groups.length === 0 && (
        <p role="alert" className="text-status-warn" data-testid="guided-no-groups">
          This token carried no groups. Group-based bindings cannot match until it does — you can
          still continue and save.
        </p>
      )}

      <div className="grid grid-cols-1 gap-2 sm:grid-cols-2">
        <ClaimPicker
          label="Username claim"
          testId="guided-username-claim"
          value={username_claim}
          names={names}
          disabled={frozen}
          onChange={(username_claim) => set({ username_claim })}
        />
        <ClaimPicker
          label="Groups claim"
          testId="guided-groups-claim"
          value={groups_claim}
          names={names}
          disabled={frozen}
          onChange={(groups_claim) => set({ groups_claim })}
        />
        <Field
          label="Username prefix"
          testId="guided-username-prefix"
          value={username_prefix ?? ''}
          hint="Must match the API server's --oidc-username-prefix, or no binding matches."
          disabled={frozen}
          onChange={(v) => set({ username_prefix: v === '' ? null : v })}
        />
        <Field
          label="Groups prefix"
          testId="guided-groups-prefix"
          value={groups_prefix ?? ''}
          hint="Must match the API server's --oidc-groups-prefix."
          disabled={frozen}
          onChange={(v) => set({ groups_prefix: v === '' ? null : v })}
        />
      </div>

      <IdentityPreview draft={draft} />
    </>
  )
}

/**
 * A claim is chosen from the claims that exist (FR-005), so a name cannot be typed from memory.
 * The current value is always an option, even when it is in neither list, so opening the step on
 * a saved issuer cannot silently re-point its claim.
 */
function ClaimPicker({
  label,
  testId,
  value,
  names,
  disabled,
  onChange,
}: {
  label: string
  testId: string
  value: string
  names: readonly string[]
  disabled?: boolean
  onChange: (v: string) => void
}) {
  if (names.length === 0) {
    return (
      <Field
        label={label}
        testId={testId}
        value={value}
        hint="No claim list to choose from yet: run the test sign-in, or fetch the discovery document."
        disabled={disabled}
        onChange={onChange}
      />
    )
  }
  const options = names.includes(value) ? names : [value, ...names]
  return (
    <label className="flex flex-col gap-1 text-xs">
      <span className="text-muted-foreground">{label}</span>
      <select
        data-testid={testId}
        className="border border-border bg-background px-2 py-1 font-mono"
        value={value}
        disabled={disabled}
        onChange={(e) => onChange(e.target.value)}
      >
        {options.map((n) => (
          <option key={n} value={n}>
            {n === '' ? '(none)' : n}
          </option>
        ))}
      </select>
    </label>
  )
}

/**
 * FR-006: the username and the group list as Kubernetes will see them, prefixes applied. The
 * groups come from the sign-in's decoded list rather than the raw claim, so empty group names
 * (Rauthy emits `[""]`) and a single-string groups claim are already what the cluster will see.
 */
function IdentityPreview({ draft }: { draft: GuidedDraft }) {
  const { username_prefix, groups_prefix, username_claim } = draft.config
  const signIn: DraftSignInResult | undefined = draft.signIn
  const claimed = signIn?.claims.find((c) => c.name === username_claim)?.value
  const username = claimed ?? signIn?.username
  const up = username_prefix ?? ''
  const gp = groups_prefix ?? ''

  if (!signIn) {
    return (
      <p className="text-muted-foreground" data-testid="guided-identity-preview-pattern">
        With these prefixes, Kubernetes will see usernames as{' '}
        <span className="font-mono">{`${up}<${username_claim}>`}</span> and groups as{' '}
        <span className="font-mono">{`${gp}<group>`}</span>. Run the test sign-in to see the real
        values.
      </p>
    )
  }
  return (
    <div className="flex flex-col gap-1" data-testid="guided-identity-preview">
      <span className="text-muted-foreground">What Kubernetes will see</span>
      <Row label="Username" value={`${up}${username ?? ''}`} testId="guided-preview-username" />
      <Row
        label="Groups"
        value={
          signIn.groups.length === 0 ? 'none' : signIn.groups.map((g) => `${gp}${g}`).join(', ')
        }
        testId="guided-preview-groups"
      />
    </div>
  )
}

/** The six fields a token's acceptance turns on, in the order FR-009 compares them. */
const COMPARED_FIELDS: readonly {
  field: string
  label: string
  cluster: (r: ApiServerAuthRead) => string | null
  draft: (c: OidcIssuerConfig) => string | null
}[] = [
  {
    field: 'issuer',
    label: 'Issuer URL',
    cluster: (r) => r.issuer,
    draft: (c) => c.issuer_url || null,
  },
  {
    field: 'client_id',
    label: 'Client ID (audience)',
    cluster: (r) => r.client_id,
    draft: (c) => c.client_id || null,
  },
  {
    field: 'username_claim',
    label: 'Username claim',
    cluster: (r) => r.username_claim,
    draft: (c) => c.username_claim || null,
  },
  {
    field: 'groups_claim',
    label: 'Groups claim',
    cluster: (r) => r.groups_claim,
    draft: (c) => c.groups_claim || null,
  },
  {
    field: 'username_prefix',
    label: 'Username prefix',
    cluster: (r) => r.username_prefix,
    draft: (c) => c.username_prefix,
  },
  {
    field: 'groups_prefix',
    label: 'Groups prefix',
    cluster: (r) => r.groups_prefix,
    draft: (c) => c.groups_prefix,
  },
]

/** What a disagreement on each field actually causes — FR-009's distinction, in words. */
const CONSEQUENCE: Record<string, string> = {
  issuer: 'Tokens from this issuer will be rejected by this API server.',
  client_id: 'Tokens for this client id will be rejected by this API server.',
  username_claim: 'The username Kubernetes sees will not be the one previewed here.',
  groups_claim: 'The groups Kubernetes sees will not be the ones previewed here.',
  username_prefix: 'Tokens are accepted, but bindings naming this user will not match.',
  groups_prefix: 'Tokens are accepted, but bindings naming these groups will not match.',
}

const STYLE_SENTENCE: Record<string, string> = {
  Legacy: 'This API server uses the legacy --oidc-* flags.',
  Structured:
    'This API server uses a structured AuthenticationConfiguration (--authentication-config); the legacy --oidc-* flags are not in use, and the two are mutually exclusive.',
  None: 'This API server accepts no OIDC tokens at all. Tokens from this issuer will be rejected until it is configured.',
  Unknown: 'Fathom could not determine how this API server authenticates.',
}

/**
 * T038 and T039. The API server's own authentication configuration beside the draft, field by
 * field (FR-008, FR-009), named with the context it was read from (FR-026), and an unreadable
 * configuration stated verbatim rather than assumed to agree (FR-010). Read-only: nothing on
 * this step writes anything, and nothing on it blocks the path.
 */
function ClusterCompareStep({ draft, activeContext }: StepProps) {
  const c: ApiServerAuthComparison | undefined = draft.comparison
  if (activeContext === undefined) {
    return (
      <p className="text-muted-foreground" data-testid="guided-compare-no-context">
        No cluster is connected, so there is nothing to compare against. Connect a context to see
        what its API server accepts; the rest of the path works without it.
      </p>
    )
  }
  if (!c) {
    return (
      <p className="text-muted-foreground" data-testid="guided-compare-pending">
        Reading how <span className="font-mono">{activeContext}</span> is configured to accept OIDC
        tokens…
      </p>
    )
  }

  const read = c.read
  const stale = read.context !== activeContext
  const differing = new Map(c.differences.map((d) => [d.field, d]))

  return (
    <>
      <p data-testid="guided-compare-context">
        Read from context <span className="font-mono">{read.context}</span>.
      </p>
      {stale && (
        <p role="alert" className="text-status-warn" data-testid="guided-compare-stale">
          The active context is now <span className="font-mono">{activeContext}</span>, so this
          comparison is stale. Re-reading it.
        </p>
      )}
      {read.checked ? (
        <p data-testid="guided-compare-style" data-style={read.style}>
          {STYLE_SENTENCE[read.style]}
        </p>
      ) : (
        <p role="alert" className="border border-border p-2" data-testid="guided-compare-unknown">
          Could not read the API server&apos;s configuration:{' '}
          <span className="font-mono break-all">{read.reason ?? 'no reason was given'}</span>. That
          is an unknown, not an agreement — nothing here is verified, and the path continues.
        </p>
      )}

      {read.checked && (
        <table className="w-full border border-border" data-testid="guided-compare-table">
          <thead>
            <tr className="text-muted-foreground">
              <th className="px-1 text-left font-normal">Field</th>
              <th className="px-1 text-left font-normal">This API server</th>
              <th className="px-1 text-left font-normal">Your draft</th>
            </tr>
          </thead>
          <tbody>
            {COMPARED_FIELDS.map((f) => {
              const diff = differing.get(f.field)
              // The backend's own row wins where it reported one, so the words the operator reads
              // are the words the comparison was made from; the accessors fill the agreeing rows.
              const cluster = diff ? diff.cluster : f.cluster(read)
              const mine = diff ? diff.draft : f.draft(draft.config)
              return (
                <tr
                  key={f.field}
                  data-testid={`guided-compare-${f.field}`}
                  data-agrees={diff ? 'false' : 'true'}
                >
                  <td className="px-1">{f.label}</td>
                  <td className="px-1 font-mono break-all">{cluster ?? 'not set'}</td>
                  <td className="px-1 font-mono break-all">{mine ?? 'not set'}</td>
                </tr>
              )
            })}
          </tbody>
        </table>
      )}
      {c.differences.length > 0 && (
        <ul className="flex flex-col gap-1" data-testid="guided-compare-consequences">
          {c.differences.map((d) => (
            <li key={d.field} data-testid={`guided-compare-consequence-${d.field}`}>
              {CONSEQUENCE[d.field] ?? `The cluster and your draft disagree on ${d.field}.`}
            </li>
          ))}
        </ul>
      )}
      {read.checked && c.differences.length === 0 && (
        <p data-testid="guided-compare-agrees">
          Every field agrees with this API server&apos;s configuration.
        </p>
      )}
      {!c.writable && (
        <p className="text-muted-foreground" data-testid="guided-compare-not-writable">
          {c.not_writable_reason ??
            'Fathom will not change a control plane it does not own. The configuration this draft needs is below, to apply yourself.'}
        </p>
      )}
      <RequiredConfiguration draft={draft} />
    </>
  )
}

/**
 * What this API server's configuration would have to say for this draft to work, in the style the
 * cluster already uses — the 011 generators, reused rather than reimplemented: `prefillFromIssuer`
 * maps the issuer onto `OidcFormInput`, `buildKubeadmFlags` renders the legacy flags and
 * `buildAuthenticationConfiguration` the structured file. Text only; Fathom applies nothing here.
 */
function RequiredConfiguration({ draft }: { draft: GuidedDraft }) {
  const input = prefillFromIssuer(draft.config)
  const legacy = draft.comparison?.read.style === 'Legacy'
  const text = legacy
    ? buildKubeadmFlags(input).join('\n')
    : buildAuthenticationConfiguration(input)
  return (
    <details className="border border-border p-2">
      <summary className="cursor-pointer text-muted-foreground">
        {legacy
          ? 'What these kube-apiserver flags would have to be'
          : 'What this API server’s AuthenticationConfiguration would have to say'}
      </summary>
      <pre
        className="mt-1 overflow-x-auto font-mono whitespace-pre"
        data-testid="guided-required-config"
      >
        {text}
      </pre>
    </details>
  )
}
