import { act, fireEvent, render, renderHook, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { Link, matchPath, navigate, setNavigationGuard, useHash, usePath, useSearchParam } from '../../src/kit'

beforeEach(() => {
  history.replaceState(null, '', '/')
})

afterEach(() => {
  setNavigationGuard(null)
})

describe('matchPath', () => {
  it('matches literals and decodes params', () => {
    expect(matchPath('/widgets', '/widgets')).toEqual({})
    expect(matchPath('/widgets/:id', '/widgets/blue%20one')).toEqual({ id: 'blue one' })
    expect(matchPath('/widgets/:id', '/widgets/blue/')).toEqual({ id: 'blue' })
  })

  it('does not match a different shape', () => {
    expect(matchPath('/widgets/:id', '/widgets')).toBeNull()
    expect(matchPath('/widgets/:id', '/gadgets/blue')).toBeNull()
    expect(matchPath('/widgets', '/widgets/blue')).toBeNull()
  })

  it('keeps the rest under * still encoded, and rejects a param it cannot decode', () => {
    expect(matchPath('/files/*', '/files/a/b%2Fc')).toEqual({ '*': 'a/b%2Fc' })
    expect(matchPath('/files/*', '/files')).toEqual({ '*': '' })
    expect(matchPath('/widgets/:id', '/widgets/%E0%A4%A')).toBeNull()
  })
})

describe('navigate', () => {
  it('pushes a same-origin path, and replaces when asked', () => {
    const length = history.length
    navigate('/widgets')
    expect(location.pathname).toBe('/widgets')
    expect(history.length).toBe(length + 1)
    navigate('/gadgets', { replace: true })
    expect(location.pathname).toBe('/gadgets')
    expect(history.length).toBe(length + 1)
  })

  it('ignores anything that is not a plain absolute path', () => {
    navigate('https://example.com/x')
    navigate('//example.com/x')
    navigate('widgets')
    expect(location.pathname).toBe('/')
  })

  it('asks the guard first, and stays put when it says no', () => {
    const guard = vi.fn(() => false)
    setNavigationGuard(guard)
    navigate('/widgets')
    expect(guard).toHaveBeenCalledTimes(1)
    expect(location.pathname).toBe('/')
    guard.mockReturnValue(true)
    navigate('/widgets')
    expect(location.pathname).toBe('/widgets')
  })
})

describe('the hooks', () => {
  it('re-render on navigation', () => {
    const { result } = renderHook(() => ({ path: usePath(), q: useSearchParam('q'), hash: useHash() }))
    expect(result.current).toEqual({ path: '/', q: null, hash: '' })
    act(() => {
      navigate('/widgets?q=blue#sharing%20it')
    })
    expect(result.current).toEqual({ path: '/widgets', q: 'blue', hash: 'sharing it' })
  })
})

describe('Link', () => {
  it('navigates in-app on a plain left click', () => {
    render(<Link to="/widgets/blue">Blue widget</Link>)
    const link = screen.getByRole('link', { name: 'Blue widget' })
    expect(link).toHaveAttribute('href', '/widgets/blue')
    const notCancelled = fireEvent.click(link)
    expect(notCancelled).toBe(false)
    expect(location.pathname).toBe('/widgets/blue')
  })

  it.each([{ ctrlKey: true }, { metaKey: true }, { shiftKey: true }, { altKey: true }, { button: 1 }])(
    'leaves %o to the browser',
    (init) => {
      render(<Link to="/widgets/blue">Blue widget</Link>)
      const notCancelled = fireEvent.click(screen.getByRole('link'), init)
      expect(notCancelled).toBe(true)
      expect(location.pathname).toBe('/')
    },
  )

  it('runs the caller’s onClick first, and stays put when it prevents the default', () => {
    const onClick = vi.fn((e: { preventDefault: () => void }) => {
      e.preventDefault()
    })
    render(
      <Link to="/widgets" onClick={onClick}>
        Widgets
      </Link>,
    )
    fireEvent.click(screen.getByRole('link'))
    expect(onClick).toHaveBeenCalledTimes(1)
    expect(location.pathname).toBe('/')
  })
})
