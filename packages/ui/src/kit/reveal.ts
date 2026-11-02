/**
 * Brings a detail panel into view when it opens (spec 060 T054). Lists open their record's detail
 * below themselves; on a phone, or under a long list, that is off screen and the click looks like
 * it did nothing. Scrolls only when the panel's top is not already on screen.
 */
import { useEffect, useRef } from 'react'

export function useReveal<T extends HTMLElement>(key: string | null) {
  const ref = useRef<T>(null)
  useEffect(() => {
    const el = ref.current
    if (key === null || !el) return
    const top = el.getBoundingClientRect().top
    if (top >= 0 && top <= window.innerHeight - 64) return
    const still = window.matchMedia('(prefers-reduced-motion: reduce)').matches
    el.scrollIntoView({ block: 'start', behavior: still ? 'auto' : 'smooth' })
  }, [key])
  return ref
}
