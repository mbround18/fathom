import { ListIcon } from '@phosphor-icons/react'

import { Button } from '@/components/ui/button'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select'
import type { WidthClass } from '@/layout/width-class'
import { useStore } from '@/store'
import { BrandMark } from '@/theme/BrandMark'

const ALL_NAMESPACES = '__all__'

export function TopBar({ widthClass }: { widthClass: WidthClass }) {
  const narrow = widthClass === 'phone' || widthClass === 'tablet'
  const setNavOpen = useStore((s) => s.setNavOpen)

  return (
    <header className="neon-chrome flex h-11 shrink-0 items-center gap-2 border-b border-border bg-card px-2 sm:px-3">
      {narrow && (
        <Button
          variant="ghost"
          size="icon"
          aria-label="Open navigator"
          onClick={() => setNavOpen(true)}
          className="size-[var(--touch-target)] shrink-0"
        >
          <ListIcon />
        </Button>
      )}
      <BrandMark compact={widthClass === 'phone'} className="shrink-0" />
      <div className="ml-auto flex min-w-0 items-center gap-2">
        <ContextPicker />
        <NamespacePicker />
      </div>
    </header>
  )
}

function ContextPicker() {
  const contexts = useStore((s) => s.session.contexts)
  const active = useStore((s) => s.session.activeContext)
  const selectContext = useStore((s) => s.selectContext)

  return (
    <Select value={active ?? ''} onValueChange={selectContext} disabled={contexts.length === 0}>
      <SelectTrigger aria-label="Context" className="w-auto max-w-[40vw] min-w-0 sm:max-w-64">
        <span className="shrink-0 text-muted-foreground">ctx</span>
        <SelectValue placeholder="none" />
      </SelectTrigger>
      <SelectContent>
        {contexts.map((c) => (
          <SelectItem key={c} value={c}>
            {c}
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  )
}

function NamespacePicker() {
  const namespaces = useStore((s) => s.session.namespaces)
  const active = useStore((s) => s.session.activeNamespace)
  const selectNamespace = useStore((s) => s.selectNamespace)

  return (
    <Select
      value={active ?? ALL_NAMESPACES}
      onValueChange={(v) => selectNamespace(v === ALL_NAMESPACES ? null : v)}
    >
      <SelectTrigger aria-label="Namespace" className="w-auto max-w-[36vw] min-w-0 sm:max-w-56">
        <span className="shrink-0 text-muted-foreground">ns</span>
        <SelectValue />
      </SelectTrigger>
      <SelectContent>
        <SelectItem value={ALL_NAMESPACES}>all</SelectItem>
        {namespaces.map((ns) => (
          <SelectItem key={ns} value={ns}>
            {ns}
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  )
}
