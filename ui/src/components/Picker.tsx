/**
 * Top-bar picker (T064): a Select for short lists, a searchable popover (cmdk) past
 * `SEARCH_THRESHOLD` options. Both expose a `combobox` trigger and `option` items.
 */
import { CaretDownIcon } from '@phosphor-icons/react'
import { Popover } from 'radix-ui'
import { useId, useState } from 'react'

import {
  Command,
  CommandEmpty,
  CommandInput,
  CommandItem,
  CommandList,
} from '@/components/ui/command'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select'
import { cn } from '@/lib/utils'

export const SEARCH_THRESHOLD = 10

export interface PickerOption {
  value: string
  label: string
}

interface PickerProps {
  label: string
  /** Short visible prefix, e.g. `ctx`. */
  prefix: string
  value: string | undefined
  options: readonly PickerOption[]
  placeholder?: string
  disabled?: boolean
  onChange: (value: string) => void
  className?: string
}

const TRIGGER =
  'h-[var(--touch-target)] w-auto min-w-0 gap-1.5 border-border/80 bg-background/40 font-mono hover:border-accent-neon/60 data-[state=open]:border-accent-neon'
const ITEM = 'min-h-[var(--touch-target)] font-mono'

interface ControlledProps extends PickerProps {
  open: boolean
  onOpenChange: (open: boolean) => void
}

export function Picker(props: PickerProps) {
  // Open state lives here so a picker that grows past the threshold while open (the namespace
  // list arriving after the operator clicked) stays open as the searchable variant.
  const [open, setOpen] = useState(false)
  return props.options.length > SEARCH_THRESHOLD ? (
    <SearchPicker {...props} open={open} onOpenChange={setOpen} />
  ) : (
    <SelectPicker {...props} open={open} onOpenChange={setOpen} />
  )
}

function Prefix({ children }: { children: string }) {
  return (
    <span className="shrink-0 font-display text-xs tracking-[0.15em] text-accent-neon/80 uppercase">
      {children}
    </span>
  )
}

function SelectPicker({
  label,
  prefix,
  value,
  options,
  placeholder,
  disabled,
  onChange,
  className,
  open,
  onOpenChange,
}: ControlledProps) {
  return (
    <Select
      value={value ?? ''}
      onValueChange={onChange}
      disabled={disabled}
      open={open}
      onOpenChange={onOpenChange}
    >
      <SelectTrigger aria-label={label} className={cn(TRIGGER, className)}>
        <Prefix>{prefix}</Prefix>
        <SelectValue placeholder={placeholder} />
      </SelectTrigger>
      <SelectContent>
        {options.map((o) => (
          <SelectItem key={o.value} value={o.value} className={ITEM}>
            {o.label}
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  )
}

function SearchPicker({
  label,
  prefix,
  value,
  options,
  placeholder,
  disabled,
  onChange,
  className,
  open,
  onOpenChange: setOpen,
}: ControlledProps) {
  const contentId = useId()
  const current = options.find((o) => o.value === value)
  return (
    <Popover.Root open={open} onOpenChange={setOpen}>
      <Popover.Trigger asChild disabled={disabled}>
        <button
          type="button"
          role="combobox"
          aria-label={label}
          aria-expanded={open}
          aria-controls={contentId}
          className={cn(
            'flex items-center border px-2 text-xs whitespace-nowrap outline-none focus-visible:ring-1 focus-visible:ring-ring disabled:opacity-50',
            TRIGGER,
            className,
          )}
        >
          <Prefix>{prefix}</Prefix>
          <span className="min-w-0 truncate">{current?.label ?? placeholder}</span>
          <CaretDownIcon aria-hidden="true" className="size-4 shrink-0 text-muted-foreground" />
        </button>
      </Popover.Trigger>
      <Popover.Portal>
        <Popover.Content
          id={contentId}
          align="end"
          sideOffset={4}
          className="z-50 w-72 max-w-[calc(100vw-2rem)] border border-accent-neon/40 bg-popover shadow-[0_0_24px_-8px_var(--accent-neon)]"
        >
          <Command>
            <CommandInput placeholder={`Find ${label.toLowerCase()}…`} />
            <CommandList>
              <CommandEmpty>No match.</CommandEmpty>
              {options.map((o) => (
                <CommandItem
                  key={o.value}
                  value={o.label}
                  data-checked={o.value === value}
                  onSelect={() => {
                    onChange(o.value)
                    setOpen(false)
                  }}
                  className={ITEM}
                >
                  {o.label}
                </CommandItem>
              ))}
            </CommandList>
          </Command>
        </Popover.Content>
      </Popover.Portal>
    </Popover.Root>
  )
}
