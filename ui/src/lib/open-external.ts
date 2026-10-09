/**
 * Open a URL outside the app. The packaged Tauri webview has nowhere to send `target="_blank"` or
 * `window.open` (no tab strip, no second window), so there the URL goes to the system browser via
 * the opener plugin; in a browser (dev server, hosted, Playwright) it is a plain new tab. Only
 * http(s) is ever handed out: the opener runs with the user's desktop session, so a `file:` or
 * custom-scheme URL from backend data must never reach it.
 */

/** The same marker `@tauri-apps/api`'s `isTauri()` checks; injected only by the Tauri webview. */
function inTauri(): boolean {
  return typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window
}

function assertHttp(url: string): void {
  let parsed: URL
  try {
    parsed = new URL(url)
  } catch {
    throw new Error(`Refusing to open a non-http(s) URL: ${url}`)
  }
  if (parsed.protocol !== 'http:' && parsed.protocol !== 'https:') {
    throw new Error(`Refusing to open a non-http(s) URL: ${url}`)
  }
}

/**
 * `onClick` for an `<a href=… target="_blank">`: the href stays for a11y and middle-click, the
 * default navigation is replaced by {@link openExternal}.
 */
export function openExternalLink(e: {
  preventDefault: () => void
  currentTarget: { href: string }
}): void {
  e.preventDefault()
  openExternal(e.currentTarget.href).catch((err: unknown) => {
    console.warn('openExternal failed', err)
  })
}

export async function openExternal(url: string): Promise<void> {
  assertHttp(url)
  if (inTauri()) {
    // Lazy: browser builds never load the plugin, which would `invoke` into a missing runtime.
    const { openUrl } = await import('@tauri-apps/plugin-opener')
    await openUrl(url)
    return
  }
  window.open(url, '_blank', 'noopener,noreferrer')
}
