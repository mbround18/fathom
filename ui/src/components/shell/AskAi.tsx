/**
 * 032-ui-shell-a11y: top-bar "Ask AI". Opens a right-hand sheet hosting the existing
 * `AgentPanel`, seeded with the current context (`useAskAiContext`) and shown as a chip in the
 * header. With no assistant configured the button stays visible but reads as unavailable and
 * offers a "Set up assistant" popover that opens the existing `AuthSetup` form.
 */
import { useFocusReturn } from '@/lib/use-focus-return'
import { SparkleIcon } from '@phosphor-icons/react'
import { Popover } from 'radix-ui'
import { useEffect } from 'react'

import { AgentPanel } from '@/components/panels/agent-ops/AgentPanel'
import { AuthSetup } from '@/components/panels/agent-ops/AuthSetup'
import { useAskAiContext } from '@/components/panels/agent-ops/askAiContext'
import {
  Button,
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
} from '@companion/ui'
import { cn } from '@companion/theme'
import { useStore } from '@/store'

/** True once the operator has chosen an assistant backend (anything but "disabled"). */
export function useAssistantConfigured(): boolean {
  return useStore(
    (s) =>
      s.agentAuth.mode !== undefined &&
      s.agentAuth.mode.type !== 'Disabled' &&
      s.agentAuth.mode.type !== 'Unconfigured',
  )
}

/** Ask the backend for the persisted auth mode once, so the button knows its own state. */
export function useLoadAssistantMode(): void {
  const request = useStore((s) => s.request)
  useEffect(() => {
    request({ type: 'AgentGetAuthMode' }).catch(() => {
      /* best-effort: the button just stays in its "set up" state */
    })
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])
}

export function AskAiButton({ compact = false }: { compact?: boolean }) {
  const configured = useAssistantConfigured()
  const setShell = useStore((s) => s.setShell)
  const icon = <SparkleIcon aria-hidden="true" data-icon="inline-start" />
  const label = compact ? <span className="sr-only">Ask AI</span> : <span>Ask AI</span>

  if (configured) {
    return (
      <Button
        variant="outline"
        size={compact ? 'icon' : 'default'}
        data-testid="ask-ai-open"
        aria-label="Ask AI"
        onClick={() => setShell({ askAiOpen: true, askAiSetup: false })}
        className={cn(
          'h-[var(--touch-target)] shrink-0 border-accent-neon/50 text-accent-neon hover:text-accent-neon',
          compact && 'w-[var(--touch-target)] px-0',
        )}
      >
        {icon}
        {!compact && label}
      </Button>
    )
  }
  return (
    <Popover.Root>
      <Popover.Trigger asChild>
        <Button
          variant="outline"
          size={compact ? 'icon' : 'default'}
          data-testid="ask-ai-open"
          aria-label="Ask AI (not set up)"
          className={cn(
            'h-[var(--touch-target)] shrink-0 border-dashed border-border/60 text-muted-foreground',
            compact && 'w-[var(--touch-target)] px-0',
          )}
        >
          {icon}
          {!compact && <span>Ask AI</span>}
        </Button>
      </Popover.Trigger>
      <Popover.Portal>
        <Popover.Content
          align="end"
          sideOffset={6}
          data-testid="ask-ai-setup-popover"
          className="z-50 flex w-72 max-w-[calc(100vw-1.5rem)] flex-col gap-2 border border-accent-neon/40 bg-popover p-3 text-xs shadow-lg"
        >
          <p className="font-display text-sm font-semibold tracking-wider uppercase">
            No assistant yet
          </p>
          <p className="text-muted-foreground">
            Connect an Anthropic key or a local Ollama model to ask questions about what you are
            looking at.
          </p>
          <Popover.Close asChild>
            <Button
              size="sm"
              data-testid="ask-ai-setup"
              onClick={() => setShell({ askAiOpen: true, askAiSetup: true })}
            >
              Set up assistant
            </Button>
          </Popover.Close>
        </Popover.Content>
      </Popover.Portal>
    </Popover.Root>
  )
}

export function AskAiSheet() {
  const open = useStore((s) => s.shell.askAiOpen)
  const setup = useStore((s) => s.shell.askAiSetup)
  const setShell = useStore((s) => s.setShell)
  const configured = useAssistantConfigured()
  const { ref, chip } = useAskAiContext()
  const showSetup = setup || !configured
  const returnFocus = useFocusReturn(open)

  return (
    <Sheet
      open={open}
      onOpenChange={(o) => setShell({ askAiOpen: o, ...(o ? {} : { askAiSetup: false }) })}
    >
      <SheetContent
        side="right"
        onCloseAutoFocus={returnFocus}
        data-testid="ask-ai-sheet"
        className="w-full max-w-full gap-0 p-0 sm:max-w-md"
      >
        <SheetHeader className="border-b border-border pr-12">
          <SheetTitle className="font-display tracking-wider uppercase">
            {showSetup ? 'Set up assistant' : 'Ask AI'}
          </SheetTitle>
          <SheetDescription className="sr-only">
            Ask the assistant about the current context.
          </SheetDescription>
          <span
            data-testid="ask-ai-context-chip"
            title={chip}
            className="mt-1 max-w-full self-start truncate border border-accent-neon/40 bg-background/40 px-2 py-0.5 font-mono text-xs text-accent-neon"
          >
            {chip}
          </span>
        </SheetHeader>
        <div className="min-h-0 flex-1 overflow-y-auto">
          {showSetup ? (
            <div className="p-3">
              <AuthSetup />
              {configured && (
                <Button
                  variant="ghost"
                  size="sm"
                  className="mt-3"
                  onClick={() => setShell({ askAiSetup: false })}
                >
                  Back to conversation
                </Button>
              )}
            </div>
          ) : (
            open && <AgentPanel key={`${ref.kind}/${ref.namespace}/${ref.name}`} object={ref} />
          )}
        </div>
      </SheetContent>
    </Sheet>
  )
}
