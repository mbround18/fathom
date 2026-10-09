/**
 * `openExternal`: the one way the UI opens a link outside its own window. In the packaged Tauri
 * app the webview has no `window.open` target, so the opener plugin hands the URL to the system
 * browser; in a browser it is a plain new tab. Only http(s) ever leaves the app.
 */
import { afterEach, beforeEach, describe, expect, it, vi, type MockInstance } from 'vitest'

const openUrl = vi.fn<(url: string) => Promise<void>>(() => Promise.resolve())
vi.mock('@tauri-apps/plugin-opener', () => ({ openUrl }))

import { openExternal } from '@/lib/open-external'

const tauriWindow = () => window as unknown as Record<string, unknown>

describe('openExternal', () => {
  let windowOpen: MockInstance<typeof window.open>

  beforeEach(() => {
    windowOpen = vi.spyOn(window, 'open').mockImplementation(() => null)
  })
  afterEach(() => {
    windowOpen.mockRestore()
    openUrl.mockClear()
    delete tauriWindow().__TAURI_INTERNALS__
  })

  it('opens a new tab in a browser', async () => {
    await openExternal('https://example.com/docs')
    expect(windowOpen).toHaveBeenCalledWith(
      'https://example.com/docs',
      '_blank',
      'noopener,noreferrer',
    )
    expect(openUrl).not.toHaveBeenCalled()
  })

  it('hands the URL to the opener plugin under Tauri', async () => {
    tauriWindow().__TAURI_INTERNALS__ = {}
    await openExternal('http://localhost:3000/')
    expect(openUrl).toHaveBeenCalledWith('http://localhost:3000/')
    expect(windowOpen).not.toHaveBeenCalled()
  })

  it.each(['javascript:alert(1)', 'file:///etc/passwd', 'ftp://host/x', 'not a url', ''])(
    'rejects %j without opening anything',
    async (url) => {
      await expect(openExternal(url)).rejects.toThrow(/http/)
      expect(windowOpen).not.toHaveBeenCalled()
      expect(openUrl).not.toHaveBeenCalled()
    },
  )
})
