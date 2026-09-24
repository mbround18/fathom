/**
 * 018-rbac-aware-mutation-ui: a `Button` that is disabled, with an explanatory tooltip, when the
 * operator's real permissions deny the action. `access.status === 'unknown'` renders a normal
 * enabled button (FR-006). The wrapper span carries the tooltip because disabled buttons swallow
 * pointer events.
 */
import type { ComponentProps } from 'react'

import { Button } from '@/components/ui/button'
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from '@/components/ui/tooltip'
import { deniedTitle, type MutationAccess } from '@/hooks/useMutationAccess'

type GatedButtonProps = ComponentProps<typeof Button> & {
  access: MutationAccess
  /** Phrase completing "You are not permitted to ...", e.g. `cordon nodes`. */
  what: string
}

export function GatedButton({ access, what, disabled, ...props }: GatedButtonProps) {
  const title = deniedTitle(access, what)
  if (title === undefined) return <Button disabled={disabled} {...props} />
  return (
    <TooltipProvider>
    <Tooltip>
      <TooltipTrigger asChild>
        <span
          data-testid="access-denied-hint"
          role="button"
          aria-disabled="true"
          aria-label={title}
          className="inline-flex"
          tabIndex={0}
        >
          <Button disabled {...props} />
        </span>
      </TooltipTrigger>
      <TooltipContent>{title}</TooltipContent>
    </Tooltip>
    </TooltipProvider>
  )
}
