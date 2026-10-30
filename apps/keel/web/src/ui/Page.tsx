/**
 * A page's frame (spec 060 T053): one padding, one gap for every section, so headers line up as
 * you move between them. `name` becomes `data-page`.
 */
import type { ReactNode } from 'react'

export function Page({ name, labelledBy, children }: { name: string; labelledBy: string; children: ReactNode }) {
  return (
    <section data-page={name} aria-labelledby={labelledBy} className="flex min-w-0 flex-col gap-6 p-4 sm:p-6">
      {children}
    </section>
  )
}
