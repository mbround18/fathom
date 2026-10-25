/**
 * Where focus goes back to when a files dialog closes (spec 053, T069). The dialogs here open from
 * a row's keys, a menu item or a store, not from a Radix `DialogTrigger`, so Radix has nothing to
 * return focus to and drops it on the page. Each dialog notes its opener when it opens and puts
 * focus back there when it closes.
 *
 * A dialog that opens as another closes (the name dialog hands over to the preview) sees focus
 * still in the old dialog, so it takes over that dialog's opener. When the opener is gone or
 * disabled (the row was deleted, the Save button is disabled once saved), focus goes to the text
 * or the row tab stop in the same pane.
 */

export interface Opener {
  el: HTMLElement
  /** The pane it sat in, for when it is gone by the time the dialog closes. */
  pane: HTMLElement | null
}

let lastOpener: Opener | null = null

/** Notes the element focus came from; call when a dialog opens. */
export function noteOpener(): Opener | null {
  const el = document.activeElement
  if (el instanceof HTMLElement && el !== document.body && el.isConnected && !el.closest('[role="dialog"], [role="menu"]')) {
    lastOpener = { el, pane: el.closest<HTMLElement>('[data-pane]') }
  }
  return lastOpener
}

const FALLBACK = '[contenteditable="true"], a[data-row-link][tabindex="0"]'

const usable = (el: HTMLElement): boolean => el.isConnected && !el.matches(':disabled') && !el.closest('[inert]')

/** Puts focus back on the opener, or near it; call when a dialog closes. */
export function returnFocus(opener: Opener | null): void {
  // Another dialog took over: it holds focus now.
  const now = document.activeElement
  if (now instanceof HTMLElement && now.isConnected && now.closest('[role="dialog"]')) return
  if (!opener) return
  if (usable(opener.el)) {
    opener.el.focus()
    return
  }
  const pane = opener.pane?.isConnected ? opener.pane : null
  pane?.querySelector<HTMLElement>(FALLBACK)?.focus()
}
