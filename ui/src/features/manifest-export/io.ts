/**
 * 040-manifest-export: copy to the clipboard and download as a file, with toasts. Both are
 * local to the browser/webview; nothing goes to the backend.
 */
import { copyText } from '@companion/ui/kit'
import { toast } from 'sonner'

/** The kit's `copyText`, with the export's toast either way. */
export function copyWithToast(text: string, what = 'YAML'): void {
  void copyText(text).then((ok) => {
    if (ok) toast.success(`Copied ${what}`, { testId: 'manifest-copy-toast' })
    else toast.error(`Could not copy ${what}`, { testId: 'manifest-copy-toast' })
  })
}

export function downloadText(text: string, filename: string, type = 'application/yaml'): void {
  const blob = new Blob([text], { type })
  const url = URL.createObjectURL(blob)
  const a = document.createElement('a')
  a.href = url
  a.download = filename
  document.body.appendChild(a)
  a.click()
  a.remove()
  // Some webviews read the blob after click returns.
  setTimeout(() => URL.revokeObjectURL(url), 1000)
  toast.success(`Downloaded ${filename}`, { testId: 'manifest-download-toast' })
}
