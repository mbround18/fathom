import { ArrowLeftIcon } from '@phosphor-icons/react'

import { Button } from '@/components/ui/button'
import { useStore } from '@/store'

import { PaneHost } from './PaneHost'
import { FORWARDS_TAB, LOGS_TAB, RESOURCES_TAB } from './stack-tabs'
import { StackTabs } from './StackTabs'

const PHONE_TABS = [RESOURCES_TAB, LOGS_TAB, FORWARDS_TAB] as const

/** Phone: one pane at a time, drill-in stack with Back, bottom tab bar (FR-012). */
export function PhoneStack() {
  const stack = useStore((s) => s.layout.stack)
  const popPane = useStore((s) => s.popPane)
  const closeInspector = useStore((s) => s.closeInspector)
  const top = stack.at(-1)
  const topIsInspector = useStore((s) => (top ? s.panes[top]?.kind === 'inspector' : false))

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      {stack.length > 1 && (
        <Button
          variant="ghost"
          onClick={topIsInspector ? closeInspector : popPane}
          className="h-[var(--touch-target)] shrink-0 justify-start border-b border-border px-3"
        >
          <ArrowLeftIcon data-icon="inline-start" /> Back
        </Button>
      )}
      <div className="min-h-0 flex-1">
        {top && <PaneHost key={top} paneId={top} slotIndex={0} />}
      </div>
      <StackTabs tabs={PHONE_TABS} withMore className="border-t" />
    </div>
  )
}
