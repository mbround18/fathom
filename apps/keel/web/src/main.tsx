import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'

import '@companion/theme/fonts.css'
import './index.css'

import { applyTheme, loadFonts } from '@companion/theme'

import { App } from '@/app/App'
import { BRAND_FONT } from '@/brand'

loadFonts(BRAND_FONT)
applyTheme({ mode: 'dark', accent: 'cyan', effects: 'on', glitch: false })

const root = document.getElementById('root')
if (!root) throw new Error('missing #root')
createRoot(root).render(
  <StrictMode>
    <App />
  </StrictMode>,
)
