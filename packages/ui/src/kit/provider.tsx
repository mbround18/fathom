/**
 * What an app hands the kit (spec 065 data-model.md, KitContext): its in-app link, how to build a
 * guide's route, which Markdown links are guides, and its glossary. Every field is optional; a piece
 * whose field is absent renders without it (no guide link, no term tip).
 */
import { createContext, useContext, type ComponentProps, type ComponentType, type ReactNode } from 'react'

export interface GuideRef {
  guide: string
  anchor?: string
}

export interface GlossaryEntry {
  term: string
  text: string
  guide?: GuideRef
}

export interface KitContext {
  /** In-app link component. Default: the kit's own `Link`. */
  Link?: ComponentType<Omit<ComponentProps<'a'>, 'href'> & { to: string }>
  /** Builds a guide's route. Absent: GuideLink renders nothing. */
  guideHref?: (ref: GuideRef) => string
  /** Tells whether a Markdown link target (`slug.md`) is one of the app's guides. */
  isGuide?: (slug: string) => boolean
  /** The bundled URL of a guide's image (`img/<name>`); absent or `undefined`: shown as a link. */
  guideImage?: (name: string) => string | undefined
  /** The app's glossary. Absent: TermTip renders nothing. */
  glossary?: Readonly<Record<string, GlossaryEntry>>
}

const Kit = createContext<KitContext>({})

export function KitProvider({ value, children }: { value: KitContext; children: ReactNode }) {
  return <Kit.Provider value={value}>{children}</Kit.Provider>
}

export function useKit(): KitContext {
  return useContext(Kit)
}
