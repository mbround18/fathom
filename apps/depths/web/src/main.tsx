import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'

import '@companion/theme/fonts.css'
import './index.css'

import { applyTheme, loadFonts } from '@companion/theme'

import { App } from '@/app/App'
import { useTheme } from '@/app/theme-store'
import { BRAND_FONT } from '@/brand'

loadFonts(BRAND_FONT)
applyTheme(useTheme.getState().theme)
useTheme.subscribe((state, prev) => {
  if (state.theme !== prev.theme) applyTheme(state.theme)
})

const root = document.getElementById('root')
if (!root) throw new Error('missing #root')
createRoot(root).render(
  <StrictMode>
    <App />
  </StrictMode>,
)
