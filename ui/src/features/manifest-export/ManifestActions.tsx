/**
 * 040-manifest-export: the Copy / Download bar shown next to any manifest (Inspector YAML tab,
 * export dialog, create/edit reviews). Test ids: `manifest-copy`, `manifest-download`, and
 * `manifest-clean` for the optional clean-for-re-apply switch.
 */
import { CopyIcon, DownloadSimpleIcon } from '@phosphor-icons/react'
import { useId } from 'react'

import { Button } from '@/components/ui/button'
import { Switch } from '@/components/ui/switch'
import { cn } from '@/lib/utils'

import { copyText, downloadText } from './io'

export interface ManifestActionsProps {
  /** The text to copy/download; `undefined` disables both (not loaded yet). */
  text: string | undefined
  filename: string
  /** Shown when set: the clean-for-re-apply switch. */
  clean?: { value: boolean; onChange: (clean: boolean) => void }
  /** Toast wording, e.g. "manifest" or "kind config". */
  what?: string
  className?: string
  children?: React.ReactNode
  /** Test id prefix, for screens that show two bars at once (default `manifest`). */
  idPrefix?: string
}

export function ManifestActions({
  text,
  filename,
  clean,
  what = 'YAML',
  className,
  children,
  idPrefix = 'manifest',
}: ManifestActionsProps) {
  const cleanId = useId()
  const disabled = text === undefined || text === ''
  return (
    <div
      role="toolbar"
      aria-label={`${what} export`}
      data-testid={`${idPrefix}-actions`}
      className={cn('flex flex-wrap items-center gap-1 text-xs', className)}
    >
      {clean && (
        <label
          htmlFor={cleanId}
          className="mr-auto flex min-h-[max(2rem,var(--touch-target))] cursor-pointer items-center gap-2 pr-2 font-mono text-muted-foreground"
          title="Strip status, managed fields, resourceVersion, uid and other server-written fields"
        >
          <Switch
            id={cleanId}
            size="sm"
            data-testid={`${idPrefix}-clean`}
            checked={clean.value}
            onCheckedChange={clean.onChange}
          />
          Clean for re-apply
        </label>
      )}
      {children}
      <Button
        variant="ghost"
        size="sm"
        type="button"
        data-testid={`${idPrefix}-copy`}
        disabled={disabled}
        onClick={() => {
          if (text) copyText(text, what)
        }}
        className="h-[max(2rem,var(--touch-target))]"
      >
        <CopyIcon aria-hidden="true" data-icon="inline-start" />
        Copy
      </Button>
      <Button
        variant="ghost"
        size="sm"
        type="button"
        data-testid={`${idPrefix}-download`}
        disabled={disabled}
        onClick={() => {
          if (text) downloadText(text, filename)
        }}
        className="h-[max(2rem,var(--touch-target))]"
      >
        <DownloadSimpleIcon aria-hidden="true" data-icon="inline-start" />
        Download
      </Button>
    </div>
  )
}
