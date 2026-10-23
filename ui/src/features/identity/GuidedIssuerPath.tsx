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
import { useEffect, useReducer, useRef, useState } from 'react'
import { cn } from 'cn'

import { ConfirmDialog } from '@/components/ConfirmDialog'
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
import { ManifestActions } from '@/features/manifest-export/ManifestActions'
import { RoleAssignmentDialog } from '@/features/roles/RoleAssignmentDialog'
import type {
  ApiServerAuthComparison,
  ApiServerAuthRead,
  ApiServerAuthStyle,
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
  canSave,
  EMPTY_ISSUER_CONFIG,
  EMPTY_ROLE_MAP,
  guidedReducer,
  initialGuidedDraft,
  isFrozen,
  issuerMismatch,
  stepsOf,
  unverified,
  type GuidedAction,
  type GuidedDraft,
  type GuidedStep,
} from './guided-state'
import { isSignedIn } from './state'

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

/** The save, which is the only write this path performs before the last two steps. */
type SaveState = { state: 'idle' } | { state: 'saving' } | { state: 'failed'; error: string }

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
  // The read for the active context when there is one, and otherwise the last read there was.
  // Hosted reads its own `in-cluster` context while the UI calls another one active, and FR-026's
  // answer to that is to render the comparison and name the context it came from — not to wait
  // for a read that will never be keyed the way this path expects.
  const storeComparison = useStore((s) => {
    const forActive =
      activeContext === undefined ? undefined : s.identity.apiServerAuth[activeContext]
    const last = s.identity.apiServerAuthLast
    return forActive ?? (last === undefined ? undefined : s.identity.apiServerAuth[last])
  })
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
  const readFor = useRef<string | undefined>(undefined)
  // The draft the read is compared against, in the effect without re-running it: a keystroke must
  // not re-ask the cluster. The read is asked once per context, so this is the draft as it stood
  // when the comparison step was entered.
  const compareDraft = useRef(draft.config)
  // Written in an effect, not during render: a ref mutated while rendering is a lint error and,
  // under StrictMode's double render, a lie about when the value changed. This effect is declared
  // before the read below, so it has already run when the read fires in the same commit.
  useEffect(() => {
    compareDraft.current = draft.config
  }, [draft.config])
  useEffect(() => {
    if (draft.step !== 'cluster-compare' || activeContext === undefined) return
    if (storeComparison?.read.context === activeContext) return
    // Asked once per context: a reply keyed to a context the UI does not call active (hosted reads
    // `in-cluster`) must not be read as "still unanswered" and asked again on every render, which
    // would be the polling Principle V forbids.
    if (readFor.current === activeContext) return
    readFor.current = activeContext
    // FR-025: the draft travels with the read. On a first-time setup nothing is saved, so without
    // it the backend has no basis for the comparison and answers `differences: []` -- which this
    // component renders as "every field agrees" about a cluster that agrees with nothing.
    send({ type: 'ReadApiServerAuth', data: { draft: compareDraft.current } })
  }, [draft.step, activeContext, storeComparison, send])

  // T049-T052: the one save. It is offered at every step (FR-015), not only on the summary, and
  // it is the only place in the path that writes anything at all.
  const savedIssuers = useStore((s) => s.identity.config.issuers)
  const liveSession = useStore((s) => isSignedIn(s.identity.identities[issuer.id]))
  const seedSource = useStore((s) => s.identity.origins[issuer.id]?.source)
  const [saveState, setSaveState] = useState<SaveState>({ state: 'idle' })
  const [confirming, setConfirming] = useState(false)

  const typedId = draft.config.id.trim()
  const reviewing = issuer.id !== ''
  // FR-017, refused here rather than only on the reply: the id keys the stored credentials, so a
  // save under a name already taken would re-point the other issuer's session.
  const duplicateId = savedIssuers.some((i) => i.id === typedId && i.id !== issuer.id)
  const idChanged = reviewing && typedId !== issuer.id
  const missing = unverified(draft)

  // What a confirmation has to say before this save happens (FR-016). An id change is the serious
  // one: the credentials are stored under the old id, so renaming is a sign-out.
  const consequences: string[] = []
  if (idChanged) {
    consequences.push(
      `The id keys the stored credentials, so saving this as ${typedId} signs ${issuer.id} out: the tokens held under ${issuer.id} are no longer reachable and you will sign in again.`,
    )
  }
  if (liveSession && !idChanged) {
    consequences.push(
      `You are signed in with ${issuer.id} right now. Saving re-points that session's configuration, and a changed issuer URL, client id or claim takes effect on its next token.`,
    )
  }

  const runSave = () => {
    setSaveState({ state: 'saving' })
    void (async () => {
      try {
        const id = await request({
          type: 'OidcSaveDraft',
          data: {
            draft: {
              config: {
                ...draft.config,
                id: typedId,
                issuer_url: draft.config.issuer_url.trim(),
                client_id: draft.config.client_id.trim(),
              },
              // The method is only how a sign-in would run; the save adopts the session the test
              // sign-in already established, so nothing here starts a new one.
              method: 'Browser',
            },
            role_map: draft.roleMap,
            // FR-028: the sign-in that was already proved becomes the operator's session rather
            // than asking for a second one. With no test sign-in there is nothing to adopt.
            adopt_test_session: draft.signIn !== undefined,
          },
        })
        const result = await waitForCommandResult(id)
        if (result === 'timeout') {
          setSaveState({ state: 'failed', error: 'The save did not complete.' })
        } else if (!result.ok) {
          // Verbatim: the backend's refusal, including FR-017's duplicate id, in its own words.
          setSaveState({ state: 'failed', error: result.error ?? 'The issuer could not be saved.' })
        } else {
          setSaveState({ state: 'idle' })
          dispatch({ type: 'saved' })
          // The two steps behind the save exist now; the bindings are the next thing to do.
          dispatch({ type: 'goto', step: 'bindings' })
        }
      } catch (err) {
        setSaveState({
          state: 'failed',
          error: err instanceof Error ? err.message : String(err),
        })
      }
    })()
  }

  return (
    <div
      className="flex min-h-0 flex-1 flex-col gap-3 overflow-y-auto px-4"
      data-testid="guided-body"
    >
      {isFrozen(draft) && (
        <p className="border border-border p-2 text-xs" data-testid="guided-seeded-origin">
          This issuer is seeded from the server&apos;s environment and is shown here, not edited: it
          is set in{' '}
          <span className="font-mono break-all">{seedSource ?? 'the server environment'}</span> and
          the server validated it before it accepted a connection. Change it there; everything below
          — the discovery document, the claims, what this cluster accepts — is still yours to read.
        </p>
      )}
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
        One section per step; `StepBody` is the only place that decides which one.

        No `min-h-0` here: the parent is the scroll container, so overriding this section's
        `min-height: auto` let it shrink below its own content. At phone height the step's last
        control then rendered *outside* the section — 17px past its bottom edge — while the
        scroller still reported nothing to scroll, so it landed on top of the next sibling and
        that sibling, painting later, swallowed the click.
      */}
      <section
        className="flex flex-1 flex-col gap-3 text-xs"
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

      {/* FR-015: the save is reachable from every step, and says what it is saving unverified. */}
      {!isFrozen(draft) && !draft.saved && (
        <div
          className="flex flex-col gap-1 border-t border-border pt-2 text-xs"
          data-testid="guided-save-bar"
        >
          {missing.length > 0 && (
            <ul
              className="flex flex-col gap-1 text-muted-foreground"
              data-testid="guided-unverified"
            >
              {missing.map((m) => (
                <li key={m}>{m}</li>
              ))}
            </ul>
          )}
          {duplicateId && (
            <p role="alert" className="text-status-err" data-testid="guided-duplicate-id">
              An issuer with the id <span className="font-mono">{typedId}</span> already exists. The
              id keys the stored credentials, so two issuers cannot share one. Choose another id.
            </p>
          )}
          {idChanged && (
            <p role="alert" className="text-status-warn" data-testid="guided-id-change-warning">
              Saving this as <span className="font-mono">{typedId}</span> signs{' '}
              <span className="font-mono">{issuer.id}</span> out: the id keys the stored
              credentials, so the tokens held under the old id are no longer reachable.
            </p>
          )}
          {liveSession && !idChanged && (
            <p role="alert" className="text-status-warn" data-testid="guided-live-session-warning">
              You are signed in with <span className="font-mono">{issuer.id}</span>. Saving changes
              the configuration that session uses.
            </p>
          )}
          {saveState.state === 'failed' && (
            <p role="alert" className="text-status-err" data-testid="guided-save-error">
              {saveState.error}
            </p>
          )}
          <div>
            <Button
              size="sm"
              data-testid="guided-save"
              disabled={!canSave(draft) || duplicateId || saveState.state === 'saving'}
              onClick={() => (consequences.length > 0 ? setConfirming(true) : runSave())}
            >
              {missing.length === 0 ? 'Save issuer' : 'Save without testing'}
            </Button>
          </div>
        </div>
      )}
      {draft.saved && (
        <p
          className="border-t border-border pt-2 text-xs text-status-ok"
          data-testid="guided-saved"
        >
          Saved as <span className="font-mono">{typedId}</span>
          {draft.signIn ? ', and signed in from the test sign-in — no second sign-in needed.' : '.'}
        </p>
      )}

      <ConfirmDialog
        open={confirming}
        title={`Save ${typedId}`}
        items={[{ namespace: null, name: typedId }]}
        detail={
          <ul className="flex flex-col gap-1" data-testid="guided-save-consequences">
            {consequences.map((c) => (
              <li key={c}>{c}</li>
            ))}
          </ul>
        }
        confirmLabel="Save issuer"
        onConfirm={() => {
          setConfirming(false)
          runSave()
        }}
        onCancel={() => setConfirming(false)}
      />

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
    case 'fathom-roles':
      return <FathomRolesStep {...p} />
    case 'review-save':
      return <ReviewSaveStep {...p} />
    case 'bindings':
      return <BindingsStep {...p} />
    case 'apiserver-write':
      return <ApiServerWriteStep {...p} />
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
        <p role="alert" className="text-status-err" data-testid="guided-probe-error">
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
          className="border border-status-err p-2"
          data-testid="guided-issuer-mismatch"
        >
          <p className="text-status-err">
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
        <p role="alert" className="text-status-err" data-testid="guided-sign-in-error">
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
function RequiredConfiguration({ draft, open }: { draft: GuidedDraft; open?: boolean }) {
  const input = prefillFromIssuer(draft.config)
  const legacy = draft.comparison?.read.style === 'Legacy'
  const text = legacy
    ? buildKubeadmFlags(input).join('\n')
    : buildAuthenticationConfiguration(input)
  return (
    <details className="border border-border p-2" open={open}>
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
      {/* T063/FR-022: on a control plane Fathom does not own this text *is* the output, so it is
          copyable with the same bar every other manifest in Fathom offers. */}
      <ManifestActions
        text={text}
        filename={legacy ? 'kube-apiserver-oidc-flags.txt' : 'authentication-config.yaml'}
        what={legacy ? 'flags' : 'configuration'}
        idPrefix="guided-config"
      />
    </details>
  )
}

/** The three slots, with what each one actually gates inside Fathom. */
const ROLE_SLOTS: readonly { key: keyof FathomRoleMap; label: string; detail: string }[] = [
  {
    key: 'viewers_group',
    label: 'Viewers group',
    detail: 'Reads what Fathom shows, and is offered no action.',
  },
  {
    key: 'editors_group',
    label: 'Editors group',
    detail: 'Is offered Fathom’s actions, each still subject to the cluster’s own RBAC.',
  },
  {
    key: 'admins_group',
    label: 'Admins group',
    detail: 'May change Fathom’s own settings, including this mapping.',
  },
]

/**
 * T042, FR-012 and the first half of FR-027. The three Fathom role slots, filled from the groups a
 * real token carried (or typed, for a group nobody has signed in with yet).
 *
 * These grant nothing in the cluster, and the step says so rather than leaving it to be inferred:
 * Kubernetes authorises on its own RoleBindings, which is the *next* step and separately refusable.
 */
function FathomRolesStep({ draft, dispatch }: StepProps) {
  const frozen = isFrozen(draft)
  const observed = draft.signIn?.groups ?? []
  const set = (patch: Partial<FathomRoleMap>) => dispatch({ type: 'edit-role-map', patch })

  return (
    <>
      <p data-testid="guided-roles-scope">
        These three groups gate <strong>Fathom&apos;s own features</strong> — what this application
        shows you and offers you. They <strong>grant nothing in the cluster</strong>: Kubernetes
        authorises on its own RoleBindings, which is a separate step you can skip entirely.
      </p>
      {observed.length === 0 && (
        <p className="text-muted-foreground" data-testid="guided-roles-no-observed">
          No test sign-in has shown which groups this provider emits, so type the group names. A
          name that matches no group simply never matches anyone.
        </p>
      )}
      <div className="flex flex-col gap-3">
        {ROLE_SLOTS.map((slot) => (
          <div key={slot.key} className="flex flex-col gap-1">
            <Field
              label={slot.label}
              testId={`guided-role-${slot.key}`}
              value={draft.roleMap[slot.key]}
              hint={slot.detail}
              disabled={frozen}
              onChange={(v) => set({ [slot.key]: v })}
            />
            {!frozen && observed.length > 0 && (
              <ul className="flex flex-wrap gap-1">
                {observed.map((g) => (
                  <li key={g}>
                    <Button
                      size="xs"
                      variant={draft.roleMap[slot.key] === g ? 'default' : 'outline'}
                      data-testid={`guided-role-option-${slot.key}-${g}`}
                      onClick={() => set({ [slot.key]: g })}
                    >
                      {g}
                    </Button>
                  </li>
                ))}
              </ul>
            )}
          </div>
        ))}
      </div>
    </>
  )
}

/** Every field the save stores, in the order the issuer is written. */
const SUMMARY_FIELDS: readonly { label: string; value: (c: OidcIssuerConfig) => string }[] = [
  { label: 'Issuer id', value: (c) => c.id },
  { label: 'Issuer URL', value: (c) => c.issuer_url },
  { label: 'Client ID', value: (c) => c.client_id },
  { label: 'Scopes', value: (c) => c.scopes.join(' ') },
  { label: 'Private CA bundle', value: (c) => c.ca_bundle_path ?? 'none' },
  { label: 'Username claim', value: (c) => c.username_claim },
  { label: 'Groups claim', value: (c) => c.groups_claim },
  { label: 'Username prefix', value: (c) => c.username_prefix ?? 'none' },
  { label: 'Groups prefix', value: (c) => c.groups_prefix ?? 'none' },
  { label: 'Admin API base', value: (c) => c.admin_api_base ?? 'none' },
]

/**
 * T050-T052. Every field that will be stored, and what was and was not verified to get here
 * (FR-014). The save itself is the bar below, which every step shows (FR-015), so this step adds
 * no second save button — there is one save, reachable from everywhere.
 */
function ReviewSaveStep({ draft }: StepProps) {
  const missing = unverified(draft)
  const done: string[] = []
  if (draft.discovery) {
    done.push(
      `The provider's discovery document was fetched and declares ${draft.discovery.issuer}.`,
    )
  }
  if (draft.signIn) {
    done.push(
      `A test sign-in succeeded as ${draft.signIn.username}, carrying ${
        draft.signIn.groups.length === 0 ? 'no groups' : draft.signIn.groups.join(', ')
      }.`,
    )
  }
  const c = draft.comparison
  if (c?.read.checked && c.differences.length === 0) {
    done.push(`Every field agrees with the API server read from ${c.read.context}.`)
  }

  return (
    <>
      <p className="text-muted-foreground">
        This is everything the save stores. Nothing has been written yet.
      </p>
      <dl className="grid grid-cols-[auto_1fr] gap-x-2 gap-y-1" data-testid="guided-summary">
        {SUMMARY_FIELDS.map((f) => (
          <div key={f.label} className="col-span-2 flex flex-wrap items-baseline gap-2">
            <dt className="text-muted-foreground">{f.label}</dt>
            <dd
              className="min-w-0 font-mono break-all"
              data-testid={`guided-summary-${f.label.toLowerCase().replaceAll(' ', '-')}`}
            >
              {f.value(draft.config) || 'not set'}
            </dd>
          </div>
        ))}
        {ROLE_SLOTS.map((slot) => (
          <div key={slot.key} className="col-span-2 flex flex-wrap items-baseline gap-2">
            <dt className="text-muted-foreground">{slot.label}</dt>
            <dd className="min-w-0 font-mono break-all" data-testid={`guided-summary-${slot.key}`}>
              {draft.roleMap[slot.key] || 'not set'}
            </dd>
          </div>
        ))}
      </dl>

      <div className="flex flex-col gap-1">
        <span className="text-muted-foreground">Verified</span>
        {done.length === 0 ? (
          <p data-testid="guided-summary-verified-none">Nothing was verified.</p>
        ) : (
          <ul className="flex flex-col gap-1" data-testid="guided-summary-verified">
            {done.map((d) => (
              <li key={d}>{d}</li>
            ))}
          </ul>
        )}
      </div>
      <div className="flex flex-col gap-1">
        <span className="text-muted-foreground">Not verified</span>
        {missing.length === 0 ? (
          <p data-testid="guided-summary-skipped-none">
            Every verification step in this path was performed.
          </p>
        ) : (
          <ul className="flex flex-col gap-1" data-testid="guided-summary-skipped">
            {missing.map((m) => (
              <li key={m}>{m}</li>
            ))}
          </ul>
        )}
      </div>
    </>
  )
}

/**
 * T044, FR-013 and the second half of FR-027: the handoff into 042's role-binding form, with the
 * issuer being configured and the group already chosen. It exists only after the save, because
 * `RoleAssignmentDraft` needs a saved `issuer_id` to resolve prefixes and the directory.
 *
 * Separately refusable: the issuer is already saved, so leaving this step creates no binding and
 * un-saves nothing. The subject is handed over as the *raw* group name — `render_subject_name` in
 * `src/backend/role_assignment.rs` applies the issuer's prefix when it renders the preview, so
 * pre-prefixing it here would produce `oidc:oidc:platform`. What the cluster will see is shown
 * beside it instead.
 */
function BindingsStep({ draft }: StepProps) {
  const namespace = useStore((s) => s.session.activeNamespace)
  const [subject, setSubject] = useState<string | undefined>(undefined)
  const gp = draft.config.groups_prefix ?? ''
  const mapped = ROLE_SLOTS.map((s) => draft.roleMap[s.key]).filter((g) => g !== '')
  const observed = draft.signIn?.groups ?? []
  const groups = [...new Set([...mapped, ...observed])]
  const prefixDiff = draft.comparison?.differences.find((d) => d.field === 'groups_prefix')

  if (subject !== undefined) {
    return (
      <div className="flex min-h-0 flex-1 flex-col gap-2" data-testid="guided-bindings-form">
        <p>
          Binding <span className="font-mono">{`${gp}${subject}`}</span> in{' '}
          <span className="font-mono">{namespace ?? 'the whole cluster'}</span>, with the issuer{' '}
          <span className="font-mono">{draft.config.id}</span>.
        </p>
        {/* 042's form, unchanged: it previews the YAML, shows the diff and applies it behind its
            own confirmation and RBAC gate. Nothing here creates a binding on its own (FR-013). */}
        <RoleAssignmentDialog
          namespace={namespace}
          issuerId={draft.config.id}
          subject={{ kind: 'Group', name: subject }}
          onClose={() => setSubject(undefined)}
        />
      </div>
    )
  }

  return (
    <>
      <p data-testid="guided-bindings-optional">
        The issuer is saved. Cluster access is separate: Kubernetes grants it through RoleBindings,
        and you can leave this step without creating any. Nothing here is undone by skipping it.
      </p>
      {prefixDiff && (
        <p role="alert" className="text-status-warn" data-testid="guided-binding-prefix-mismatch">
          This API server uses the groups prefix{' '}
          <span className="font-mono">{prefixDiff.cluster ?? 'none'}</span> while this issuer uses{' '}
          <span className="font-mono">{prefixDiff.draft ?? 'none'}</span>, so a binding written with
          this issuer&apos;s prefix will not match the groups the cluster sees.
        </p>
      )}
      {groups.length === 0 ? (
        <p className="text-muted-foreground" data-testid="guided-bindings-no-groups">
          No group to bind: nothing was mapped to a Fathom slot and no test sign-in showed a group.
          The role-binding screens can still bind one by hand.
        </p>
      ) : (
        <ul className="flex flex-col gap-1">
          {groups.map((g) => (
            <li key={g} className="flex flex-wrap items-center gap-2">
              <Button
                size="xs"
                variant="outline"
                data-testid={`guided-binding-subject-${g}`}
                onClick={() => setSubject(g)}
              >
                Bind {g}…
              </Button>
              <span className="font-mono text-muted-foreground">Kubernetes sees {`${gp}${g}`}</span>
            </li>
          ))}
        </ul>
      )}
    </>
  )
}

/** The write, which only ever runs against a control plane Fathom provisioned. */
type WriteState =
  { state: 'idle' } | { state: 'writing' } | { state: 'failed'; error: string } | { state: 'done' }

/**
 * T062 and T063. On a cluster Fathom provisioned, the API server's authentication configuration
 * written in the style it already uses (FR-019, FR-020) behind a confirmation that names the exact
 * change and the restart. On every other context there is no write at all, only the same
 * configuration as copyable text, with the refusal in the backend's own words (FR-022).
 */
function ApiServerWriteStep({ draft }: StepProps) {
  const request = useStore((s) => s.request)
  const [confirming, setConfirming] = useState(false)
  const [write, setWrite] = useState<WriteState>({ state: 'idle' })
  const c = draft.comparison

  if (!c) {
    return (
      <p className="text-muted-foreground" data-testid="guided-write-unread">
        This API server&apos;s configuration has not been read, so there is nothing to change. Go
        back to <em>What this cluster accepts</em> first.
      </p>
    )
  }

  if (!c.writable) {
    return (
      <>
        <p role="alert" data-testid="guided-write-refused">
          {c.not_writable_reason ??
            'Fathom will not change a control plane it did not provision. Apply this yourself with your cluster tooling.'}
        </p>
        <p className="text-muted-foreground">
          Here is what <span className="font-mono break-all">{c.read.context}</span> would have to
          say for this issuer to be accepted. Copy it; Fathom applies nothing here.
        </p>
        <RequiredConfiguration draft={draft} open />
      </>
    )
  }

  const input = prefillFromIssuer(draft.config)
  // FR-020: whichever style the cluster already uses, never both. With no OIDC configuration at
  // all there is nothing to match, so the legacy flags are written — the style kubeadm, and so
  // every cluster Fathom provisions with kind, starts from.
  const style: ApiServerAuthStyle = c.read.style === 'Structured' ? 'Structured' : 'Legacy'
  const text =
    style === 'Structured'
      ? buildAuthenticationConfiguration(input)
      : buildKubeadmFlags(input).join('\n')
  // `WriteApiServerAuth` names the cluster Fathom provisioned; the read names the kubeconfig
  // context, which kind writes as `kind-<cluster>`.
  const cluster = c.read.context.startsWith('kind-')
    ? c.read.context.slice('kind-'.length)
    : c.read.context

  const runWrite = () => {
    setWrite({ state: 'writing' })
    void (async () => {
      try {
        const id = await request({
          type: 'WriteApiServerAuth',
          data: {
            cluster,
            style,
            issuer_url: draft.config.issuer_url.trim(),
            client_id: draft.config.client_id.trim(),
            username_claim: draft.config.username_claim,
            groups_claim: draft.config.groups_claim,
            username_prefix: draft.config.username_prefix,
            groups_prefix: draft.config.groups_prefix,
          },
        })
        const result = await waitForCommandResult(id)
        if (result === 'timeout') {
          setWrite({ state: 'failed', error: 'The API server did not report back.' })
        } else if (!result.ok) {
          // Verbatim, including the backup's location and how to restore it (FR-021). Fathom does
          // not retry on its own: the next attempt is the operator's.
          setWrite({ state: 'failed', error: result.error ?? 'The write failed.' })
        } else {
          setWrite({ state: 'done' })
        }
      } catch (err) {
        setWrite({ state: 'failed', error: err instanceof Error ? err.message : String(err) })
      }
    })()
  }

  return (
    <>
      <p data-testid="guided-write-target">
        This changes how the API server of <span className="font-mono">{cluster}</span>{' '}
        authenticates — the cluster Fathom provisioned, read from context{' '}
        <span className="font-mono break-all">{c.read.context}</span>.
      </p>
      <p data-testid="guided-write-style">
        {style === 'Structured'
          ? 'This API server already uses a structured AuthenticationConfiguration, so the file is replaced. The legacy --oidc-* flags are mutually exclusive with it and are not added.'
          : c.read.style === 'Legacy'
            ? 'This API server already uses the legacy --oidc-* flags, so those flags are set. No AuthenticationConfiguration is added: the two are mutually exclusive.'
            : 'This API server accepts no OIDC tokens yet, so the legacy --oidc-* flags are set — the style kubeadm, and so every kind cluster, starts from. No AuthenticationConfiguration is added; adding one would require removing these flags.'}
      </p>
      <p className="text-muted-foreground" data-testid="guided-write-backup">
        Before changing anything, Fathom copies the current{' '}
        <span className="font-mono">/etc/kubernetes/manifests/kube-apiserver.yaml</span> aside on
        the control-plane node, then waits for the API server to come back. If it does not, the
        error, the backup&apos;s path and how to restore it are shown here, and Fathom does not
        retry on its own.
      </p>
      <pre
        className="overflow-x-auto border border-border p-2 font-mono whitespace-pre"
        data-testid="guided-write-change"
      >
        {text}
      </pre>
      <ManifestActions
        text={text}
        filename={
          style === 'Structured' ? 'authentication-config.yaml' : 'kube-apiserver-oidc-flags.txt'
        }
        what={style === 'Structured' ? 'configuration' : 'flags'}
        idPrefix="guided-write-config"
      />
      {write.state === 'failed' && (
        <p
          role="alert"
          className="font-mono break-words text-status-err"
          data-testid="guided-write-error"
        >
          {write.error}
        </p>
      )}
      {write.state === 'done' && (
        <p className="text-status-ok" data-testid="guided-write-done">
          Written, and the API server came back. It is in the mutation audit log like every other
          cluster write.
        </p>
      )}
      <div>
        <Button
          size="sm"
          data-testid="guided-write-apply"
          disabled={write.state === 'writing' || write.state === 'done'}
          onClick={() => setConfirming(true)}
        >
          {style === 'Structured'
            ? 'Write the AuthenticationConfiguration…'
            : 'Set these kube-apiserver flags…'}
        </Button>
      </div>

      <ConfirmDialog
        open={confirming}
        title={`Change how ${cluster} authenticates`}
        items={[{ namespace: null, name: cluster }]}
        detail={
          <div className="flex flex-col gap-2" data-testid="guided-write-confirm-detail">
            <p>
              The API server of <span className="font-mono">{cluster}</span> restarts. Until it is
              back, nothing can reach that cluster — including Fathom.
            </p>
            <pre className="max-h-40 overflow-auto font-mono text-xs whitespace-pre">{text}</pre>
          </div>
        }
        confirmLabel="Write and restart the API server"
        onConfirm={() => {
          setConfirming(false)
          runWrite()
        }}
        onCancel={() => setConfirming(false)}
      />
    </>
  )
}
