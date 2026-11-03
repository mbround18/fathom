/**
 * A field for typing or pasting a secret: hidden as typed, with Show to check it before saving.
 * Browsers are told not to remember, autocomplete or spell-check it.
 */
import { EyeIcon, EyeSlashIcon } from '@phosphor-icons/react'
import { useState, type ComponentProps } from 'react'

import { Button } from '../button'
import { FIELD } from './form'

export function SecretInput({
  className = '',
  ...props
}: Omit<ComponentProps<'input'>, 'type' | 'autoComplete' | 'spellCheck'> & { id: string }) {
  const [shown, setShown] = useState(false)
  return (
    <div className={`flex min-w-0 items-center gap-1 ${className}`}>
      <input
        {...props}
        type={shown ? 'text' : 'password'}
        autoComplete="off"
        autoCapitalize="off"
        autoCorrect="off"
        spellCheck={false}
        data-1p-ignore
        data-lpignore="true"
        className={`${FIELD} flex-1 font-mono`}
      />
      <Button
        type="button"
        variant="ghost"
        size="xs"
        aria-controls={props.id}
        onClick={() => {
          setShown((s) => !s)
        }}
      >
        {shown ? <EyeSlashIcon aria-hidden /> : <EyeIcon aria-hidden />}
        {shown ? 'Hide' : 'Show'}
      </Button>
    </div>
  )
}
