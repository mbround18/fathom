import type { ThemePrefs } from '@/protocol/types'

/**
 * Reflect theme prefs onto <html> (contracts/theme.md): `.dark` for neon-noir (absent = light
 * blueprint), plus `data-accent`, `data-effects`, and `data-glitch` for the CSS in tokens.css and
 * effects.css. Idempotent; call it whenever `store.ui.theme` changes.
 */
export function applyTheme(theme: ThemePrefs, root: HTMLElement = document.documentElement): void {
  root.classList.toggle('dark', theme.mode === 'dark')
  root.dataset.accent = theme.accent
  root.dataset.effects = theme.effects
  root.dataset.glitch = theme.glitch ? 'on' : 'off'
}
