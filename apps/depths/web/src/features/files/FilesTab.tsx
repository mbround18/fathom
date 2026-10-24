/**
 * The stack page's Files tab (spec 053): the volume explorer. Lazy-loaded by `StackDetail` at
 * `/stacks/:name/files/*`, only for people the tab is shown to (`showFilesTab`); the server gates
 * every command regardless.
 *
 * The URL is the state (FR-024): `useFiles.go` follows it. Layout (FR-023), by the width the tab
 * gets (a container query, since the stack list takes a column beside it on wide screens):
 * - narrow (a phone): one pane at a time: the mount list, then a folder, then the file full width
 *   with a way back to its folder;
 * - `@3xl` (768 px): the folder and the open file side by side;
 * - `@6xl` (1152 px): the mount tree as well, so three panes.
 */
import { useEffect } from 'react'

import { useSession } from '@/app/session-store'

import { FileEditor } from './FileEditor'
import { FileOpDialog } from './FileOpDialog'
import { Listing } from './Listing'
import { filesMode, type FilesLocation } from './model'
import { MountList } from './MountList'
import { useFiles } from './store'

export default function FilesTab({ stack, location }: { stack: string; location: FilesLocation }) {
  const go = useFiles((s) => s.go)
  useEffect(() => {
    go(stack, location)
  }, [go, stack, location.mount, location.path]) // eslint-disable-line react-hooks/exhaustive-deps

  const mode = useSession((s) => filesMode(s.me))
  const ours = useFiles((s) => s.stack === stack)
  const mounts = useFiles((s) => (ours ? s.mounts : null))
  const mount = useFiles((s) => s.mount)
  const dir = useFiles((s) => s.dir)
  const entries = useFiles((s) => s.entries)
  const list = useFiles((s) => s.list)
  const listRefusal = useFiles((s) => s.listRefusal)
  const filter = useFiles((s) => s.filter)
  const setFilter = useFiles((s) => s.setFilter)
  const file = useFiles((s) => s.file)

  const here = location.mount
  const info = mounts?.find((m) => m.id === here)
  const rootLabel = info?.source ?? here ?? ''
  const inMount = ours && here !== null && mount === here
  const open = inMount && file.state !== 'closed' ? file : null

  return (
    <section
      aria-label={`Files of ${stack}`}
      data-tab="files"
      data-mount={here ?? ''}
      data-path={location.path}
      className="@container flex min-w-0 flex-col gap-1 lg:min-h-0 lg:flex-1"
    >
      {here === null ? (
        <MountList stack={stack} mounts={mounts} />
      ) : (
        <div className="flex min-w-0 flex-col gap-3 @3xl:flex-row @3xl:items-start">
          <aside aria-label="Mounts" className="hidden w-64 shrink-0 @6xl:block">
            <MountList stack={stack} mounts={mounts} current={here} compact />
          </aside>
          <div className={`min-w-0 flex-col @3xl:flex @3xl:flex-1 ${open ? 'hidden' : 'flex'}`}>
            <Listing
              stack={stack}
              mount={here}
              rootLabel={rootLabel}
              dir={inMount ? dir : ''}
              entries={inMount ? entries : []}
              list={inMount ? list : 'loading'}
              refusal={inMount ? listRefusal : null}
              filter={filter}
              onFilter={setFilter}
              openPath={open?.path ?? null}
              canChange={mode === 'full' && info?.writable !== false}
            />
          </div>
          {open && (
            <div className="min-w-0 @3xl:flex-[1.4]">
              <FileEditor stack={stack} mount={here} mountInfo={info} rootLabel={rootLabel} file={open} mode={mode} />
            </div>
          )}
        </div>
      )}
      <FileOpDialog />
    </section>
  )
}
