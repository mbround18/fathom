import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'

import '@companion/theme/fonts.css'
import './index.css'

import { applyTheme, loadFonts } from '@companion/theme'

import { BRAND_FONT } from '@/brand'
import { useStore } from '@/store'

import App from './App'

loadFonts(BRAND_FONT)
applyTheme(useStore.getState().ui.theme)
useStore.subscribe((state, prev) => {
  if (state.ui.theme !== prev.ui.theme) applyTheme(state.ui.theme)
})

const root = document.getElementById('root')
if (!root) throw new Error('#root is missing from index.html')

createRoot(root).render(
  <StrictMode>
    <App />
  </StrictMode>,
)
