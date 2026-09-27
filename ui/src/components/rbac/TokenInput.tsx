/**
 * A chip list plus a free-text input with suggestions (042-role-create). Used by the rules editor
 * for API groups, resources and resource names: the suggestions come from API discovery, but any
 * text is accepted so CRDs discovery has not seen yet (or wildcards like `*`) still work.
 *
 * Enter, comma or blur commits the typed text (comma/space separated, so pasting `pods,services`
 * adds two tokens); Backspace on an empty input removes the last chip.
 */
import { useId, useState, type KeyboardEvent } from 'react'

import { XIcon } from '@phosphor-icons/react'

import { Input } from '@/components/ui/input'

import { parseTokens } from './roleYaml'

export interface TokenInputProps {
  label: string
  values: readonly string[]
  onChange: (next: string[]) => void
  suggestions?: readonly string[]
  placeholder?: string
  /** Treat `core`/`""` as the core API group (`''`) and show `''` as `core`. */
  coreGroup?: boolean
  testId: string
}

export function TokenInput({
  label,
  values,
  onChange,
  suggestions = [],
  placeholder,
  coreGroup = false,
  testId,
}: TokenInputProps) {
  const [draft, setDraft] = useState('')
  const listId = useId()
  const display = (v: string) => (coreGroup && v === '' ? 'core' : v)

  const commit = () => {
    const tokens = parseTokens(draft, coreGroup)
    setDraft('')
    if (tokens.length === 0) return
    const next = [...values]
    for (const t of tokens) if (!next.includes(t)) next.push(t)
    onChange(next)
  }

  const onKeyDown = (e: KeyboardEvent<HTMLInputElement>) => {
    if (e.key === 'Enter' || e.key === ',') {
      e.preventDefault()
      commit()
    } else if (e.key === 'Backspace' && draft === '' && values.length > 0) {
      onChange(values.slice(0, -1))
    }
  }

  return (
    <div className="flex min-w-0 flex-col gap-1 text-xs">
      <span>{label}</span>
      <div className="flex flex-wrap items-center gap-1">
        {values.map((v) => (
          <span
            key={v}
            data-testid={`${testId}-token`}
            className="inline-flex items-center gap-1 border border-border bg-muted/40 px-1.5 py-0.5 font-mono"
          >
            {display(v)}
            <button
              type="button"
              aria-label={`Remove ${display(v)}`}
              className="text-muted-foreground hover:text-foreground"
              onClick={() => onChange(values.filter((x) => x !== v))}
            >
              <XIcon className="size-3" />
            </button>
          </span>
        ))}
        <Input
          aria-label={label}
          data-testid={`${testId}-input`}
          list={listId}
          placeholder={placeholder}
          className="h-7 w-40 min-w-0 flex-1"
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          onKeyDown={onKeyDown}
          onBlur={commit}
        />
        <datalist id={listId}>
          {suggestions
            .filter((s) => !values.includes(s))
            .map((s) => (
              <option key={s || '(core)'} value={display(s)} />
            ))}
        </datalist>
      </div>
    </div>
  )
}
