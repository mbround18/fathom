/**
 * 040-manifest-export: copy to the clipboard and download as a file, with toasts. Both are
 * local to the browser/webview; nothing goes to the backend.
 */
import { toast } from 'sonner'

export function copyText(text: string, what = 'YAML'): void {
  const done = () => toast.success(`Copied ${what}`, { testId: 'manifest-copy-toast' })
  const fail = () => toast.error(`Could not copy ${what}`, { testId: 'manifest-copy-toast' })
  try {
    void navigator.clipboard.writeText(text).then(done, fail)
  } catch {
    fail()
  }
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
