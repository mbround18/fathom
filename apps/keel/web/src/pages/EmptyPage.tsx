/**
 * A page that has no content yet (spec 055 T030): the title and a line on what will live here.
 * Each story replaces its page with the real one.
 */
export function EmptyPage({ page, title, blurb }: { page: string; title: string; blurb: string }) {
  return (
    <section data-page={page} aria-labelledby={`${page}-title`} className="flex flex-col gap-2 p-4">
      <h1 id={`${page}-title`} className="font-display text-sm font-semibold tracking-wider uppercase">
        {title}
      </h1>
      <p data-pane-state="empty" className="text-xs text-muted-foreground">
        {blurb}
      </p>
    </section>
  )
}
