/**
 * A page's frame (spec 060 T053): one padding, one gap for every section, so headers line up as
 * you move between them. `name` becomes `data-page`.
 */
import type { ReactNode } from 'react'

export function Page({ name, labelledBy, children }: { name: string; labelledBy: string; children: ReactNode }) {
  return (
    <section data-page={name} aria-labelledby={labelledBy} className="flex min-w-0 flex-col gap-4 p-3 sm:p-4">
      {children}
    </section>
  )
}
