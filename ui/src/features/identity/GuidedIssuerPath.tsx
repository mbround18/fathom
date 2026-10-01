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
import { useReducer } from 'react'
import { cn } from 'cn'

import { Button } from '@/components/ui/button'
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetFooter,
  SheetHeader,
  SheetTitle,
} from '@/components/ui/sheet'
import type { FathomRoleMap, IssuerOrigin, OidcIssuerConfig } from '@/protocol/types'
import { useStore } from '@/store'

import {
  canAdvance,
  EMPTY_ISSUER_CONFIG,
  EMPTY_ROLE_MAP,
  guidedReducer,
  initialGuidedDraft,
  stepsOf,
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
        One section per step, filled by the tasks that own them: T023 (URL, discovery), T030/T031
        (claims, prefixes), T038 (comparison), T042 (role groups), T050 (summary), T044 (bindings),
        T062 (API server write).
      */}
      <section
        className="min-h-0 flex-1"
        aria-label={STEP_TITLES[draft.step]}
        data-testid={`guided-step-body-${draft.step}`}
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
