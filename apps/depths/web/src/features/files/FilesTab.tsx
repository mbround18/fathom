/**
 * The stack page's Files tab (spec 053): the volume explorer. Lazy-loaded by `StackDetail` at
 * `/stacks/:name/files/*`, only for people the tab is shown to (`showFilesTab`); the server gates
 * every command regardless. For now an empty state: the mount list, listing and editor land here
 * in US1 (T035–T038).
 */
import type { FilesLocation } from './model'

export default function FilesTab({ stack, location }: { stack: string; location: FilesLocation }) {
  return (
    <section
      aria-label={`Files of ${stack}`}
      data-tab="files"
      data-mount={location.mount ?? ''}
      data-path={location.path}
      className="flex min-w-0 flex-col gap-1"
    >
      <p role="status" data-pane-state="empty" className="text-xs text-muted-foreground">
        No mounts yet.
      </p>
    </section>
  )
}
