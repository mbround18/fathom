import { StrictMode, useEffect, type ReactNode } from 'react'
import { createRoot } from 'react-dom/client'

import '@companion/theme/fonts.css'
import './harness.css'

import { applyTheme } from '@companion/theme'

import { KitProvider, type GlossaryEntry } from '../src/kit'
import { PIECES } from './pieces'

declare global {
  interface Window {
    __kitReady?: boolean
  }
}

/** Plausible, app-free terms for the TermTip fixtures. */
const HARNESS_GLOSSARY: Readonly<Record<string, GlossaryEntry>> = {
  widget: { term: 'Widget', text: 'A thing the harness lists, so a piece has something to show.', guide: { guide: 'widgets' } },
  gadget: { term: 'Gadget', text: 'Another thing, with no guide.' },
}

const params = new URLSearchParams(location.search)
const theme = params.get('theme') === 'light' ? 'light' : 'dark'
applyTheme({ mode: theme, accent: 'cyan', effects: 'on', glitch: false })

function Ready({ children }: { children: ReactNode }) {
  useEffect(() => {
    void document.fonts.ready.then(() => {
      window.__kitReady = true
    })
  }, [])
  return children
}

/** The index: every piece, or one piece's states, as links. The seed of the later gallery. */
function Index({ title, links }: { title: string; links: readonly { href: string; label: string }[] }) {
  return (
    <>
      <h1 className="mb-3 text-xl font-semibold">{title}</h1>
      <ul className="flex flex-col gap-1">
        {links.map((l) => (
          <li key={l.href}>
            <a className="text-primary underline underline-offset-2" href={l.href}>
              {l.label}
            </a>
          </li>
        ))}
      </ul>
    </>
  )
}

function Harness() {
  const id = params.get('piece')
  const state = params.get('state')
  const piece = PIECES.find((p) => p.id === id)
  const q = (extra: Record<string, string>) => `?${new URLSearchParams({ theme, ...extra }).toString()}`

  let body: ReactNode
  if (!piece) {
    body = <Index title="Kit pieces" links={PIECES.map((p) => ({ href: q({ piece: p.id }), label: p.id }))} />
  } else {
    const render = state ? piece.states[state] : undefined
    body = render ? (
      render()
    ) : (
      <Index title={piece.id} links={Object.keys(piece.states).map((s) => ({ href: q({ piece: piece.id, state: s }), label: s }))} />
    )
  }

  return (
    <KitProvider
      value={{
        guideHref: (r) => `/help/${r.guide}${r.anchor ? `#${r.anchor}` : ''}`,
        isGuide: (s) => s === 'example',
        glossary: HARNESS_GLOSSARY,
      }}
    >
      <main aria-label="Harness" className="p-4">
        <Ready>{body}</Ready>
      </main>
    </KitProvider>
  )
}

const root = document.getElementById('root')
if (!root) throw new Error('missing #root')
createRoot(root).render(
  <StrictMode>
    <Harness />
  </StrictMode>,
)
