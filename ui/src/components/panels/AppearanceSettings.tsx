/**
 * Appearance, keyboard shortcuts and About (T117). Theme changes go through `setTheme` /
 * `setDensityOverride`; main.tsx re-applies the theme via `applyTheme` and the store persists
 * with a debounced `SaveUiConfig`.
 */
import type { Density, ThemeAccent, ThemeEffects, ThemeMode } from '@/protocol/types'
import { useStore } from '@/store'

const SHORTCUTS: readonly (readonly [string, string])[] = [
  ['Ctrl/Cmd + K', 'Open the command palette'],
  ['Esc', 'Close the inspector or dialog'],
]

function Select<T extends string>({
  label,
  value,
  options,
  onChange,
}: {
  label: string
  value: T
  options: readonly T[]
  onChange: (v: T) => void
}) {
  return (
    <label className="flex items-center gap-2 text-xs">
      <span className="w-20">{label}</span>
      <select
        aria-label={label}
        className="border border-border bg-background px-2 py-1"
        value={value}
        onChange={(e) => onChange(e.target.value as T)}
      >
        {options.map((o) => (
          <option key={o} value={o}>
            {o}
          </option>
        ))}
      </select>
    </label>
  )
}

export function AppearanceSettings() {
  const theme = useStore((s) => s.ui.theme)
  const density = useStore((s) => s.ui.density.override)
  const setTheme = useStore((s) => s.setTheme)
  const setDensityOverride = useStore((s) => s.setDensityOverride)
  return (
    <fieldset className="flex flex-col gap-2" data-testid="appearance-settings">
      <legend className="text-xs font-semibold tracking-wider uppercase">Appearance</legend>
      <Select<ThemeMode>
        label="Mode"
        value={theme.mode}
        options={['dark', 'light']}
        onChange={(mode) => setTheme({ mode })}
      />
      <Select<ThemeAccent>
        label="Accent"
        value={theme.accent}
        options={['cyan', 'magenta']}
        onChange={(accent) => setTheme({ accent })}
      />
      <Select<ThemeEffects>
        label="Effects"
        value={theme.effects}
        options={['on', 'off']}
        onChange={(effects) => setTheme({ effects })}
      />
      <Select<Density | 'auto'>
        label="Density"
        value={density ?? 'auto'}
        options={['auto', 'comfortable', 'compact']}
        onChange={(d) => setDensityOverride(d === 'auto' ? undefined : d)}
      />
      <label className="flex items-center gap-2 text-xs">
        <input
          type="checkbox"
          checked={theme.glitch}
          onChange={(e) => setTheme({ glitch: e.target.checked })}
        />
        Glitch effect
      </label>

      <h3 className="mt-2 text-xs font-semibold tracking-wider uppercase">Keyboard shortcuts</h3>
      <dl className="grid grid-cols-[auto_1fr] gap-x-4 text-xs" data-testid="shortcuts">
        {SHORTCUTS.map(([keys, what]) => (
          <div key={keys} className="contents">
            <dt className="font-mono">{keys}</dt>
            <dd>{what}</dd>
          </div>
        ))}
      </dl>

      <h3 className="mt-2 text-xs font-semibold tracking-wider uppercase">About</h3>
      <p className="text-xs" data-testid="about">
        Fathom (desktop shell) <span data-testid="about-version">v{__APP_VERSION__}</span>
      </p>
    </fieldset>
  )
}
