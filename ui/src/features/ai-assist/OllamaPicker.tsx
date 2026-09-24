import { useVirtualizer } from '@tanstack/react-virtual'
import { useMemo, useRef, useState } from 'react'

import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import type { OllamaModel } from '@/protocol/types'
import { useStore } from '@/store'

const VIRTUALIZE_ABOVE = 50
const ROW_PX = 36

function fmtSize(bytes: number): string {
  return `${(bytes / 1e9).toFixed(1)} GB`
}

function ModelList({
  models,
  selected,
  onPick,
}: {
  models: OllamaModel[]
  selected: string | null
  onPick: (m: string) => void
}) {
  const ref = useRef<HTMLDivElement>(null)
  const virtual = models.length > VIRTUALIZE_ABOVE
  const v = useVirtualizer({
    count: models.length,
    getScrollElement: () => ref.current,
    estimateSize: () => ROW_PX,
    overscan: 8,
  })
  const row = (m: OllamaModel) => (
    <button
      key={m.name}
      type="button"
      role="option"
      aria-selected={selected === m.name}
      data-testid={`ollama-model-${m.name}`}
      onClick={() => onPick(m.name)}
      className={`flex h-9 w-full items-center justify-between px-2 text-left text-xs ${selected === m.name ? 'bg-accent' : ''}`}
    >
      <span>{m.name}</span>
      <span className="text-muted-foreground">
        {[m.parameter_size, fmtSize(m.size_bytes)].filter(Boolean).join(' · ')}
      </span>
    </button>
  )
  return (
    <div
      ref={ref}
      role="listbox"
      aria-label="Ollama models"
      className="max-h-64 overflow-y-auto rounded border"
    >
      {virtual ? (
        <div style={{ height: v.getTotalSize(), position: 'relative' }}>
          {v.getVirtualItems().map((it) => (
            <div
              key={it.key}
              style={{ position: 'absolute', top: 0, width: '100%', transform: `translateY(${it.start}px)` }}
            >
              {row(models.at(it.index) as OllamaModel)}
            </div>
          ))}
        </div>
      ) : (
        models.map(row)
      )}
    </div>
  )
}

export function OllamaPicker() {
  const request = useStore((s) => s.request)
  const state = useStore((s) => s.ai.ollama)
  const selected = useStore((s) => s.ai.ollamaModel)
  const [q, setQ] = useState('')
  const filtered = useMemo(
    () => (state?.models ?? []).filter((m) => m.name.toLowerCase().includes(q.toLowerCase())),
    [state, q],
  )
  const probe = () => void request({ type: 'AiOllamaProbe', data: { endpoint: null } })

  let body
  if (state === undefined) {
    body = <p className="text-muted-foreground text-xs">Not checked yet.</p>
  } else if (!state.reachable) {
    body = (
      <div data-testid="ollama-not-running" className="text-xs">
        <p>
          Ollama is not running at {state.endpoint}.
          {state.installed_hint === 'found'
            ? ' It looks installed — start it with `ollama serve`.'
            : state.installed_hint === 'not_found'
              ? ' It does not appear to be installed (ollama.com).'
              : ''}
        </p>
        {state.error !== null && <p className="text-muted-foreground">{state.error}</p>}
      </div>
    )
  } else if (state.models.length === 0) {
    body = (
      <p data-testid="ollama-no-models" className="text-xs">
        Ollama {state.version ?? ''} is running but has no models. Run `ollama pull llama3.2`.
      </p>
    )
  } else {
    body = (
      <>
        <Input
          type="search"
          value={q}
          onChange={(e) => setQ(e.target.value)}
          placeholder="Search models"
          aria-label="Search Ollama models"
        />
        <ModelList
          models={filtered}
          selected={selected}
          onPick={(model) => void request({ type: 'AiOllamaSelectModel', data: { model } })}
        />
      </>
    )
  }

  return (
    <div data-testid="ollama-picker" className="flex flex-col gap-2">
      <div className="flex items-center justify-between">
        <span className="font-medium">Ollama (local)</span>
        <Button size="sm" variant="outline" data-testid="ollama-recheck" onClick={probe}>
          Re-check
        </Button>
      </div>
      {body}
    </div>
  )
}
