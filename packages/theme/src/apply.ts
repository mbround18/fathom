/**
 * Theme prefs as the stylesheet understands them. Structural, so each app can pass its own
 * protocol type (Fathom's generated `ThemePrefs`, Depths' own) without this package importing it.
 */
export interface ThemePrefs {
  mode: 'dark' | 'light'
  accent: 'cyan' | 'magenta'
  effects: 'on' | 'off'
  glitch: boolean
}

/**
 * Reflect theme prefs onto <html> (contracts/theme.md): `.dark` for neon-noir (absent = light
 * blueprint), plus `data-accent`, `data-effects`, and `data-glitch` for the CSS in tokens.css and
 * effects.css. Idempotent; call it whenever the app's theme prefs change.
 */
export function applyTheme(theme: ThemePrefs, root: HTMLElement = document.documentElement): void {
  root.classList.toggle('dark', theme.mode === 'dark')
  root.dataset.accent = theme.accent
  root.dataset.effects = theme.effects
  root.dataset.glitch = theme.glitch ? 'on' : 'off'
}
