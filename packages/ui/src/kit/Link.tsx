import type { ComponentProps, MouseEvent } from 'react'

import { navigate } from './router'

/** An `<a>` that navigates in-app on a plain left click and behaves natively otherwise. */
export function Link({ to, onClick, children, ...rest }: Omit<ComponentProps<'a'>, 'href'> & { to: string }) {
  const handle = (e: MouseEvent<HTMLAnchorElement>): void => {
    onClick?.(e)
    if (e.defaultPrevented || e.button !== 0 || e.metaKey || e.ctrlKey || e.shiftKey || e.altKey)
      return
    e.preventDefault()
    navigate(to)
  }
  return (
    <a href={to} onClick={handle} {...rest}>
      {children}
    </a>
  )
}
