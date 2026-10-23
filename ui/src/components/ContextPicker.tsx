import { ExportIcon } from '@phosphor-icons/react'
import { useMemo } from 'react'

import { Button } from '@companion/ui'
import { useKubeconfigExport } from '@/features/manifest-export/dialog-stores'
import { cn } from '@companion/theme'
import { transportKind } from '@/protocol/transport'
import { useStore } from '@/store'

import { Picker } from './Picker'

/** Kubeconfig context switcher (T064). Switching resets every list (FR-016). */
export function ContextPicker({ className }: { className?: string }) {
  const contexts = useStore((s) => s.session.contexts)
  const active = useStore((s) => s.session.activeContext)
  const selectContext = useStore((s) => s.selectContext)
  const hosted = useStore((s) => s.hosted.mode === 'hosted') || transportKind() === 'hosted'
  const showExport = useKubeconfigExport((s) => s.show)
  const open = useStore((s) => s.shell.contextPickerOpen)
  const setShell = useStore((s) => s.setShell)
  const options = useMemo(() => contexts.map((c) => ({ value: c, label: c })), [contexts])
  const picker = (
    <Picker
      label="Context"
      prefix="ctx"
      value={active}
      options={options}
      placeholder="none"
      disabled={contexts.length === 0}
      onChange={selectContext}
      open={open}
      onOpenChange={(o) => setShell({ contextPickerOpen: o })}
      className={hosted ? className : 'min-w-0 flex-1'}
    />
  )
  // 040 US4 (FR-021, FR-031): kubeconfig export is a desktop feature, never offered hosted.
  if (hosted) return picker
  return (
    <div className={cn('flex min-w-0 items-center', className)}>
      {picker}
      <Button
        variant="ghost"
        size="icon"
        aria-label="Export kubeconfig…"
        title="Export kubeconfig…"
        data-testid="kubeconfig-export-open"
        disabled={contexts.length === 0}
        onClick={() => showExport(active ?? null)}
        className="size-[max(2rem,var(--touch-target))] shrink-0 text-muted-foreground hover:text-accent-neon"
      >
        <ExportIcon aria-hidden="true" />
      </Button>
    </div>
  )
}
