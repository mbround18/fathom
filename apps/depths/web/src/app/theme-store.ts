/**
 * Theme prefs (neon-noir by default). Kept per browser in `localStorage` under `depths:theme`:
 * a display preference, never anything about the session.
 */
import type { ThemePrefs } from '@companion/theme'
import { create } from 'zustand'

const KEY = 'depths:theme'

export const DEFAULT_THEME: ThemePrefs = {
  mode: 'dark',
  accent: 'cyan',
  effects: 'on',
  glitch: false,
}

function load(): ThemePrefs {
  try {
    const raw = globalThis.localStorage.getItem(KEY)
    if (!raw) return DEFAULT_THEME
    const v = JSON.parse(raw) as Partial<ThemePrefs>
    return {
      mode: v.mode === 'light' ? 'light' : 'dark',
      accent: v.accent === 'magenta' ? 'magenta' : 'cyan',
      effects: v.effects === 'off' ? 'off' : 'on',
      glitch: v.glitch === true,
    }
  } catch {
    return DEFAULT_THEME
  }
}

function save(theme: ThemePrefs): void {
  try {
    globalThis.localStorage.setItem(KEY, JSON.stringify(theme))
  } catch {
    /* Private mode or blocked storage: the pref lasts for this page only. */
  }
}

interface ThemeState {
  theme: ThemePrefs
  toggleMode: () => void
}

export const useTheme = create<ThemeState>()((set, get) => ({
  theme: load(),
  toggleMode: () => {
    const cur = get().theme
    const theme: ThemePrefs = { ...cur, mode: cur.mode === 'dark' ? 'light' : 'dark' }
    save(theme)
    set({ theme })
  },
}))
