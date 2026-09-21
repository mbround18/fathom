import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'

import '@/theme/fonts'
import './index.css'

import { useStore } from '@/store'
import { applyTheme } from '@/theme/apply'

import App from './App'

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
