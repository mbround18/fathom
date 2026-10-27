/**
 * Keel's web shell (spec 055). Phase 1 scaffolding: the shell title only; sign-in, the socket and
 * the feature pages arrive in the Foundational phase.
 */
import { BRAND_GLYPH, BRAND_NAME } from '@/brand'

export function App() {
  return (
    <main className="flex h-full flex-col items-center justify-center gap-2">
      <h1 className="text-2xl">
        <span aria-hidden="true" className="mr-2">
          {BRAND_GLYPH}
        </span>
        {BRAND_NAME}
      </h1>
    </main>
  )
}
