/// <reference types="vite/client" />

interface ImportMetaEnv {
  /** `fake` selects the scripted in-page backend (`pnpm dev:fake`); default is Tauri IPC. */
  readonly VITE_TRANSPORT?: 'tauri' | 'fake' | 'ws' | 'hosted'
  /** `ws` mode fallback bridge URL; prefer `?bridge=…&token=…` so no token is bundled. */
  readonly VITE_DEVBRIDGE_URL?: string
}

/** App version from src-tauri/tauri.conf.json, injected at build time (vite.config.ts). */
declare const __APP_VERSION__: string
