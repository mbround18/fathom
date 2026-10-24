/**
 * A folder of a mount (spec 053, T036): a breadcrumb that stops at the mount root, a name filter,
 * and the entries, virtualized so a folder of 50,000 names stays fast (SC-002).
 *
 * - Columns: name, size, modified, owner (`uid:gid`), permissions (`rwxr-x---`). On a narrow pane
 *   the details wrap under the name instead of scrolling sideways.
 * - Folders, files and links that stay inside the mount open; a link out of the mount shows its
 *   target and does not open, and a special file (FIFO, socket, device) is listed, inert.
 * - Keyboard: the rows are one tab stop; ArrowUp/ArrowDown/Home/End move between the rows that
 *   open, Enter opens. Rows are at least 28 px high for touch.
 */
import { FileIcon, FolderIcon, LinkSimpleIcon, MagnifyingGlassIcon, ProhibitIcon } from '@phosphor-icons/react'
import { useMemo, useRef, useState, type KeyboardEvent } from 'react'
import { useVirtualizer } from '@tanstack/react-virtual'

import { Link } from '@/app/Link'
import { formatBytes } from '@/features/stacks/metrics'
import { formatTime } from '@/features/stacks/model'
import type { Entry } from '@/protocol/generated'

import { filesHref, modeString, parentPath, pathParts } from './model'
import { entryPath, filterEntries, refusalText, type FilesRefusal, type ListStatus } from './store'

const ESTIMATED_ROW_PX = 40
const OVERSCAN = 10

export interface ListingProps {
  stack: string
  mount: string
  /** The root crumb: the volume name or host path. */
  rootLabel: string
  dir: string
  entries: readonly Entry[]
  list: ListStatus
  refusal: FilesRefusal | null
  filter: string
  onFilter: (filter: string) => void
  /** The open file, marked in the listing. */
  openPath: string | null
}

/** Whether a row opens: folders, files and links that stay inside. */
function opens(e: Entry): boolean {
  return e.kind === 'dir' || e.kind === 'file' || (e.kind === 'link' && (e.link?.inside ?? false))
}

const count = (n: number): string => n.toLocaleString('en-US')

export function Listing({ stack, mount, rootLabel, dir, entries, list, refusal, filter, onFilter, openPath }: ListingProps) {
  const shown = useMemo(() => filterEntries(entries, filter), [entries, filter])
  const filtered = shown !== entries
  const at = (path: string): string => filesHref(stack, { mount, path })

  const scrollRef = useRef<HTMLDivElement>(null)
  // The app does not run the React Compiler; the virtualizer is read fresh every render.
  // eslint-disable-next-line react-hooks/incompatible-library
  const virtualizer = useVirtualizer({
    count: shown.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => ESTIMATED_ROW_PX,
    getItemKey: (i) => shown[i]?.name ?? i,
    overscan: OVERSCAN,
  })
  const virtual = virtualizer.getVirtualItems()
  const total = virtualizer.getTotalSize()
  const first = virtual[0]
  const last = virtual[virtual.length - 1]

  // The roving tab stop: the row Tab lands on.
  const [active, setActive] = useState(0)
  const firstOpen = shown.findIndex(opens)
  const tabStop = shown[active] && opens(shown[active]) ? active : firstOpen

  const focusRow = (i: number): void => {
    setActive(i)
    virtualizer.scrollToIndex(i, { align: 'auto' })
    const tryFocus = (left: number): void => {
      const a = scrollRef.current?.querySelector<HTMLAnchorElement>(`[data-index="${String(i)}"] a[data-row-link]`)
      if (a) a.focus()
      else if (left > 0) requestAnimationFrame(() => tryFocus(left - 1))
    }
    tryFocus(5)
  }

  const onKeyDown = (ev: KeyboardEvent<HTMLAnchorElement>): void => {
    const row = (ev.target as HTMLElement).closest<HTMLElement>('[data-index]')
    if (!row) return
    const from = Number(row.dataset.index)
    let to = -1
    if (ev.key === 'ArrowDown') {
      for (let i = from + 1; i < shown.length; i++) if (opens(shown[i] as Entry)) { to = i; break }
    } else if (ev.key === 'ArrowUp') {
      for (let i = from - 1; i >= 0; i--) if (opens(shown[i] as Entry)) { to = i; break }
    } else if (ev.key === 'Home') {
      to = firstOpen
    } else if (ev.key === 'End') {
      for (let i = shown.length - 1; i >= 0; i--) if (opens(shown[i] as Entry)) { to = i; break }
    } else return
    ev.preventDefault()
    if (to >= 0) focusRow(to)
  }

  const parts = pathParts(dir) ?? []

  return (
    <div data-pane="listing" className="@container flex min-w-0 flex-col gap-1">
      <div className="flex min-w-0 flex-wrap items-center gap-2">
        <nav aria-label="Folder path" className="min-w-0 flex-1">
          <ol className="flex min-w-0 flex-wrap items-center gap-x-1 text-xs">
            {[rootLabel, ...parts].map((label, i) => {
              const current = i === parts.length
              return (
                <li key={i} className="flex min-w-0 items-center gap-1">
                  {i > 0 && <span aria-hidden className="text-muted-foreground">/</span>}
                  {current ? (
                    <span aria-current="page" className="min-w-0 font-mono font-medium break-all">
                      {label}
                    </span>
                  ) : (
                    <Link to={at(parts.slice(0, i).join('/'))} className="min-w-0 font-mono break-all text-primary hover:underline">
                      {label}
                    </Link>
                  )}
                </li>
              )
            })}
          </ol>
        </nav>
        {dir !== '' && (
          <Link
            to={at(parentPath(dir))}
            aria-label="Up one folder"
            className="inline-flex h-7 shrink-0 items-center border border-input px-2 text-xs hover:border-primary hover:text-primary"
          >
            Up
          </Link>
        )}
      </div>

      <div className="flex min-w-0 flex-wrap items-center gap-2">
        <label className="flex h-7 min-w-0 flex-1 basis-40 items-center gap-1 border border-input bg-background px-1.5 focus-within:border-ring">
          <MagnifyingGlassIcon aria-hidden className="size-3.5 shrink-0 text-muted-foreground" />
          <input
            type="search"
            aria-label="Filter this folder"
            placeholder="Filter this folder"
            value={filter}
            onChange={(e) => {
              onFilter(e.target.value)
              setActive(0)
            }}
            className="h-full min-w-0 flex-1 bg-transparent text-xs outline-none placeholder:text-muted-foreground"
          />
        </label>
        <span data-entry-count className="shrink-0 text-xs text-muted-foreground tabular-nums">
          {list === 'ready' || list === 'loading'
            ? filtered
              ? `${count(shown.length)} of ${count(entries.length)}`
              : `${count(entries.length)} ${entries.length === 1 ? 'entry' : 'entries'}${list === 'loading' ? ' so far' : ''}`
            : ''}
        </span>
      </div>

      {list === 'refused' && refusal ? (
        <p role="alert" data-refusal={refusal.kind} className="border border-border p-2 text-xs break-words text-status-err">
          {refusalText(refusal)}
        </p>
      ) : list === 'loading' && entries.length === 0 ? (
        <p role="status" data-pane-state="loading" className="border border-border p-2 text-xs text-muted-foreground">
          Loading…
        </p>
      ) : shown.length === 0 && list === 'ready' ? (
        <p role="status" data-pane-state="empty" className="border border-border p-2 text-xs text-muted-foreground">
          {filtered ? 'No names match the filter.' : 'This folder is empty.'}
        </p>
      ) : (
        <div
          ref={scrollRef}
          role="list"
          aria-label="Entries"
          className="h-[60vh] min-h-48 min-w-0 overflow-x-hidden overflow-y-auto border border-border lg:h-[calc(100dvh-16rem)]"
        >
          <div style={{ paddingTop: first?.start ?? 0, paddingBottom: last ? total - last.end : 0 }}>
            {virtual.map((v) => {
              const e = shown[v.index]
              if (!e) return null
              const path = entryPath(dir, e)
              return (
                <div key={v.key} data-index={v.index} ref={virtualizer.measureElement} role="listitem">
                  <Row entry={e} href={at(path)} current={path === openPath} tabIndex={v.index === tabStop ? 0 : -1} onKeyDown={onKeyDown} />
                </div>
              )
            })}
          </div>
        </div>
      )}
    </div>
  )
}

interface RowProps {
  entry: Entry
  href: string
  current: boolean
  tabIndex: number
  onKeyDown: (ev: KeyboardEvent<HTMLAnchorElement>) => void
}

function Row({ entry: e, href, current, tabIndex, onKeyDown }: RowProps) {
  const Icon = e.kind === 'dir' ? FolderIcon : e.kind === 'link' ? LinkSimpleIcon : e.kind === 'special' ? ProhibitIcon : FileIcon
  const name = (
    <>
      <Icon aria-hidden className={`size-3.5 shrink-0 ${e.kind === 'dir' ? 'text-primary' : 'text-muted-foreground'}`} />
      <span className="min-w-0 font-mono break-all">{e.name}</span>
    </>
  )
  return (
    <div
      data-entry={e.name}
      data-kind={e.kind}
      aria-current={current ? 'true' : undefined}
      className={`grid min-h-7 min-w-0 grid-cols-[minmax(0,1fr)_auto] items-center gap-x-2 border-b border-border px-2 py-1 text-xs @xl:grid-cols-[minmax(0,1fr)_5rem_8.5rem_5.5rem_5.5rem] ${current ? 'bg-accent' : ''}`}
    >
      <div className="flex min-w-0 flex-col">
        {opens(e) ? (
          <Link to={href} data-row-link tabIndex={tabIndex} onKeyDown={onKeyDown} className="flex min-h-6 min-w-0 items-center gap-1.5 outline-none hover:text-primary focus-visible:ring-1 focus-visible:ring-ring">
            {name}
          </Link>
        ) : (
          <span className="flex min-h-6 min-w-0 items-center gap-1.5 text-muted-foreground">{name}</span>
        )}
        {e.kind === 'link' && e.link && (
          <span data-field="link-target" className="min-w-0 pl-5 font-mono break-all text-muted-foreground">
            → {e.link.target}
            {!e.link.inside && <span className="ml-1 text-status-warn">(outside the mount)</span>}
          </span>
        )}
        {e.kind === 'special' && <span className="pl-5 text-muted-foreground">special file, does not open</span>}
        <span className="pl-5 text-muted-foreground @xl:hidden">
          {formatTime(e.mtime)} · {e.uid}:{e.gid} · <span className="font-mono">{modeString(e.mode)}</span>
        </span>
      </div>
      <span data-field="size" className="text-right text-muted-foreground tabular-nums">
        {e.kind === 'dir' ? '—' : formatBytes(e.size)}
      </span>
      <span data-field="mtime" className="hidden text-muted-foreground tabular-nums @xl:block">
        {formatTime(e.mtime)}
      </span>
      <span data-field="owner" className="hidden truncate text-muted-foreground @xl:block">
        {e.uid}:{e.gid}
      </span>
      <span data-field="mode" className="hidden font-mono text-muted-foreground @xl:block">
        {modeString(e.mode)}
      </span>
    </div>
  )
}
