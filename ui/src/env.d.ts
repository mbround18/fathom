/// <reference types="vite/client" />

interface ImportMetaEnv {
  /** `fake` selects the scripted in-page backend (`pnpm dev:fake`); default is Tauri IPC. */
  readonly VITE_TRANSPORT?: 'tauri' | 'fake'
}
