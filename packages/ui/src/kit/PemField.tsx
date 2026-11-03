/**
 * A field for a certificate, a chain or a key in PEM: paste it or load it from a file. It says
 * what it found ("2 blocks: CERTIFICATE, CERTIFICATE") or what is wrong, and refuses blocks of a
 * type it does not take, so a private key pasted where a certificate goes is caught here.
 */
import { UploadSimpleIcon } from '@phosphor-icons/react'
import { useId, useRef, type ReactNode } from 'react'

import { Button } from '../button'
import { FIELD_AREA, LABEL } from './form'
import { checkPem, describePem } from './pem'

export function PemField({
  id,
  label,
  value,
  onChange,
  expect,
  hint,
  accept = '.pem,.crt,.cer,.key,.csr',
}: {
  id: string
  label: ReactNode
  value: string
  onChange: (value: string) => void
  /** Block types the field takes, e.g. `['CERTIFICATE']`; any type when left out. */
  expect?: readonly string[]
  hint?: ReactNode
  accept?: string
}) {
  const file = useRef<HTMLInputElement>(null)
  const status = useId()
  const check = value.trim() === '' ? null : checkPem(value, expect)
  return (
    <div className="flex min-w-0 flex-col gap-1">
      <div className="flex min-w-0 items-center gap-2">
        <label htmlFor={id} className={`${LABEL} flex-1`}>
          {label}
        </label>
        <Button
          type="button"
          variant="outline"
          size="xs"
          onClick={() => {
            file.current?.click()
          }}
        >
          <UploadSimpleIcon aria-hidden />
          Load from file
        </Button>
        <input
          ref={file}
          type="file"
          accept={accept}
          hidden
          tabIndex={-1}
          aria-hidden
          onChange={(e) => {
            const f = e.target.files?.[0]
            e.target.value = ''
            if (f) void f.text().then(onChange)
          }}
        />
      </div>
      <textarea
        id={id}
        value={value}
        rows={6}
        autoComplete="off"
        autoCapitalize="off"
        autoCorrect="off"
        spellCheck={false}
        placeholder="-----BEGIN …-----"
        aria-invalid={check?.ok === false}
        aria-describedby={status}
        className={`${FIELD_AREA} w-full resize-y font-mono text-xs`}
        onChange={(e) => {
          onChange(e.target.value)
        }}
      />
      <p id={status} data-pem-status={check === null ? 'empty' : check.ok ? 'ok' : 'error'} className="text-xs text-muted-foreground">
        {check === null ? hint : check.ok ? describePem(check.blocks) : <span className="text-status-err">{check.error}</span>}
      </p>
    </div>
  )
}
