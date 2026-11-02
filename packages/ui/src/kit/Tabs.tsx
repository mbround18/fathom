/**
 * Tabs (spec 060 R10): `@companion/ui`'s radix tabs in the line style, controlled. The tab strip
 * scrolls sideways inside itself on a narrow screen, never the page. Only the active panel is
 * rendered: pass it as `children`.
 */
import { Tabs as Root, TabsContent, TabsList, TabsTrigger } from '../tabs'
import type { ReactNode } from 'react'

export interface TabItem<T extends string> {
  value: T
  label: string
  icon?: ReactNode
}

export function Tabs<T extends string>({
  label,
  items,
  value,
  onChange,
  children,
}: {
  label: string
  items: readonly TabItem<T>[]
  value: T
  onChange: (value: T) => void
  children: ReactNode
}) {
  return (
    <Root
      value={value}
      onValueChange={(v) => {
        const item = items.find((i) => i.value === v)
        if (item) onChange(item.value)
      }}
      className="min-w-0 gap-3"
    >
      <div className="min-w-0 overflow-x-auto border-b border-border">
        <TabsList variant="line" aria-label={label} className="h-9 min-w-full justify-start">
          {items.map((i) => (
            <TabsTrigger key={i.value} value={i.value} data-tab={i.value} className="flex-none px-2.5">
              {i.icon}
              {i.label}
            </TabsTrigger>
          ))}
        </TabsList>
      </div>
      <TabsContent value={value} className="min-w-0">
        {children}
      </TabsContent>
    </Root>
  )
}
