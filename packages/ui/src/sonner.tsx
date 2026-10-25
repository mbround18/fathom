import { CheckCircleIcon, InfoIcon, SpinnerIcon, WarningIcon, XCircleIcon } from '@phosphor-icons/react'
import type { CSSProperties } from 'react'
import { Toaster as Sonner, type ToasterProps as SonnerProps } from 'sonner'

type ToasterProps = Omit<SonnerProps, 'theme'> & {
  /** The app's current theme mode; the app owns `.dark`, so there is no next-themes here. */
  theme: 'dark' | 'light'
}

/** Toasts follow the app theme, which the app passes in from its own store. */
function Toaster({ theme, ...props }: ToasterProps) {
  return (
    <Sonner
      theme={theme}
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
      toastOptions={{
        classNames: {
          toast: 'cn-toast notch font-mono',
          description: 'whitespace-pre-line break-words',
        },
      }}
      {...props}
    />
  )
}

export { Toaster, type ToasterProps }
