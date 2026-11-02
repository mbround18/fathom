import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'

import '@companion/theme/fonts.css'
import './index.css'

import { applyTheme, loadFonts } from '@companion/theme'
import { KitProvider, type KitContext } from '@companion/ui/kit'

import { BRAND_FONT } from '@/brand'
import { useStore } from '@/store'

import App from './App'

loadFonts(BRAND_FONT)
applyTheme(useStore.getState().ui.theme)
useStore.subscribe((state, prev) => {
  if (state.ui.theme !== prev.ui.theme) applyTheme(state.ui.theme)
})

/**
 * Spec 065: the shared kit's context. Fathom has no in-app guides or glossary yet, so every field
 * is left out: GuideLink and TermTip render nothing, and links use the kit's own.
 */
const KIT: KitContext = {}

const root = document.getElementById('root')
if (!root) throw new Error('#root is missing from index.html')

createRoot(root).render(
  <StrictMode>
    <KitProvider value={KIT}>
      <App />
    </KitProvider>
  </StrictMode>,
)
