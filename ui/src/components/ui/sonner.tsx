import { CheckCircleIcon, InfoIcon, SpinnerIcon, WarningIcon, XCircleIcon } from '@phosphor-icons/react'
import type { CSSProperties } from 'react'
import { Toaster as Sonner, type ToasterProps } from 'sonner'

import { useStore } from '@/store'

/** Toasts follow the app theme from the store (no next-themes; the shell owns `.dark`). */
function Toaster(props: ToasterProps) {
  const mode = useStore((s) => s.ui.theme.mode)

  return (
    <Sonner
      theme={mode}
      className="toaster group"
      icons={{
        success: <CheckCircleIcon className="size-4 text-status-ok" />,
        info: <InfoIcon className="size-4 text-accent-neon" />,
        warning: <WarningIcon className="size-4 text-status-warn" />,
        error: <XCircleIcon className="size-4 text-status-err" />,
        loading: <SpinnerIcon className="size-4 animate-spin" />,
      }}
      style={
        {
          '--normal-bg': 'var(--popover)',
          '--normal-text': 'var(--popover-foreground)',
          '--normal-border': 'var(--border)',
          '--border-radius': 'var(--radius)',
        } as CSSProperties
      }
      toastOptions={{ classNames: { toast: 'cn-toast notch font-mono' } }}
      {...props}
    />
  )
}

export { Toaster }
