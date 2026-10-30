"use client"

import * as React from "react"
import { cn } from "cn"
import { InfoIcon } from "@phosphor-icons/react"
import { Popover as PopoverPrimitive } from "radix-ui"

// A toggletip: a small info button that explains a term. Unlike Tooltip it opens on tap as well
// as hover and focus, and its content can hold a link (the footer). Focus stays on the trigger;
// while open the trigger is described by the content, so a screen reader reads the explanation.

type DataAttributes = { [key: `data-${string}`]: string | undefined }

type InfoTipProps = {
  /** The term being explained. The trigger's accessible name is "About {label}". */
  label: string
  /** The explanation. */
  children: React.ReactNode
  /** Optional line under the explanation, e.g. a "Learn more" link. */
  footer?: React.ReactNode
  /** Classes for the trigger. */
  className?: string
  /** `data-*` attributes for the popover content (the rest of the `data-*` props go on the trigger). */
  contentData?: DataAttributes
} & DataAttributes

// Grace period for the pointer to travel from the trigger to the content (or back).
const LEAVE_DELAY_MS = 120

function InfoTip({ label, children, footer, className, contentData, ...data }: InfoTipProps) {
  const [open, setOpen] = React.useState(false)
  // Opened by click or tap: mouse leave no longer closes it.
  const [pinned, setPinned] = React.useState(false)
  const contentId = React.useId()
  const contentRef = React.useRef<HTMLDivElement>(null)
  const leaveTimer = React.useRef<ReturnType<typeof setTimeout> | undefined>(undefined)

  const cancelLeave = React.useCallback(() => {
    if (leaveTimer.current !== undefined) {
      clearTimeout(leaveTimer.current)
      leaveTimer.current = undefined
    }
  }, [])

  const close = React.useCallback(() => {
    cancelLeave()
    setOpen(false)
    setPinned(false)
  }, [cancelLeave])

  const scheduleLeave = React.useCallback(
    (event: React.PointerEvent) => {
      if (event.pointerType !== "mouse" || pinned) return
      cancelLeave()
      leaveTimer.current = setTimeout(() => {
        leaveTimer.current = undefined
        setOpen(false)
      }, LEAVE_DELAY_MS)
    },
    [pinned, cancelLeave]
  )

  React.useEffect(() => cancelLeave, [cancelLeave])

  return (
    <PopoverPrimitive.Root
      open={open}
      onOpenChange={(next) => {
        // Radix calls this for Escape and a pointer-down outside.
        if (next) setOpen(true)
        else close()
      }}
    >
      <PopoverPrimitive.Trigger
        data-slot="info-tip-trigger"
        aria-label={`About ${label}`}
        aria-haspopup={undefined}
        aria-controls={open ? contentId : undefined}
        aria-describedby={open ? contentId : undefined}
        className={cn(
          // 24×24 hit area around a 14px icon; the negative margin keeps it from pushing text.
          "-m-[5px] inline-flex size-6 shrink-0 cursor-help items-center justify-center rounded-none align-middle text-muted-foreground transition-colors outline-none hover:text-foreground focus-visible:text-foreground focus-visible:ring-2 focus-visible:ring-ring/50 data-[state=open]:text-foreground",
          className
        )}
        {...data}
        onPointerEnter={(event) => {
          if (event.pointerType !== "mouse") return
          cancelLeave()
          setOpen(true)
        }}
        onPointerLeave={scheduleLeave}
        onFocus={() => setOpen(true)}
        onBlur={(event) => {
          const next = event.relatedTarget
          if (next instanceof Node && contentRef.current?.contains(next)) return
          close()
        }}
        onClick={(event) => {
          // Handle the toggle here instead of Radix's: a click that follows the focus- or
          // hover-open should pin the tip open, not close it.
          event.preventDefault()
          cancelLeave()
          if (open && pinned) {
            close()
          } else {
            setOpen(true)
            setPinned(true)
          }
        }}
      >
        <InfoIcon aria-hidden="true" className="size-3.5" />
      </PopoverPrimitive.Trigger>
      <PopoverPrimitive.Portal>
        <PopoverPrimitive.Content
          ref={contentRef}
          id={contentId}
          role="note"
          data-slot="info-tip-content"
          side="top"
          align="start"
          sideOffset={4}
          collisionPadding={16}
          onOpenAutoFocus={(event) => event.preventDefault()}
          // Focus never left the trigger, so there is nothing to give back. Radix would refocus the
          // trigger on close, and that focus would open a hover-opened tip again.
          onCloseAutoFocus={(event) => event.preventDefault()}
          onPointerEnter={(event) => {
            if (event.pointerType === "mouse") cancelLeave()
          }}
          onPointerLeave={scheduleLeave}
          className={cn(
            "z-50 flex w-max max-w-[min(20rem,calc(100vw-2rem))] origin-(--radix-popover-content-transform-origin) flex-col gap-1.5 rounded-none border border-border bg-popover px-3 py-2 text-xs/relaxed text-popover-foreground shadow-md outline-none data-[side=bottom]:slide-in-from-top-2 data-[side=left]:slide-in-from-right-2 data-[side=right]:slide-in-from-left-2 data-[side=top]:slide-in-from-bottom-2 data-open:animate-in data-open:fade-in-0 data-open:zoom-in-95 data-closed:animate-out data-closed:fade-out-0 data-closed:zoom-out-95 motion-reduce:animate-none"
          )}
          {...contentData}
        >
          <div>{children}</div>
          {footer ? <div data-slot="info-tip-footer">{footer}</div> : null}
        </PopoverPrimitive.Content>
      </PopoverPrimitive.Portal>
    </PopoverPrimitive.Root>
  )
}

export { InfoTip }
export type { InfoTipProps }
