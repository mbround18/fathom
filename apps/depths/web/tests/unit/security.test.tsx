import { fireEvent, render, screen } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import { SecurityChip } from '@/features/stacks/SecurityChip'
import { chipFor, reasonLines, rootfulNotice, shellGate, worstFinding } from '@/features/stacks/security'
import type { ComposeWarning, ContainerSecurity, Features } from '@/protocol/generated'

const features: Features = { shell: true, edit: true, host_metrics: false }

function rated(over: Partial<ContainerSecurity> = {}): ContainerSecurity {
  return {
    level: 'root',
    reasons: [
      { code: 'root_user', severity: 'root', message: 'Runs as root (uid 0) on a rootful daemon.' },
      { code: 'net_admin', severity: 'info', message: 'Has NET_ADMIN.' },
    ],
    shell: 'admin_only',
    ...over,
  }
}

function warning(severity: ComposeWarning['severity'], code = 'x'): ComposeWarning {
  return { service: 'app', code, severity, message: code, line: 3 }
}

describe('051 security helpers', () => {
  it('chips root amber and escape red, and nothing for ok or unrated', () => {
    expect(chipFor('root')?.tone).toContain('status-warn')
    expect(chipFor('escape')?.tone).toContain('status-err')
    expect(chipFor('ok')).toBeNull()
    expect(chipFor(undefined)).toBeNull()
    expect(chipFor(null)).toBeNull()
  })

  it('says the reasons worst first, leaving out the informational ones', () => {
    expect(reasonLines(rated())).toEqual(['Runs as root (uid 0) on a rootful daemon.'])
    expect(reasonLines(undefined)).toEqual([])
  })

  it('gates the shell button by the server rule and the role, never by a missing rating', () => {
    expect(shellGate(rated({ shell: 'refused', level: 'escape' }), 'admin')).toMatchObject({ enabled: false })
    expect(shellGate(rated({ shell: 'admin_only' }), 'user')).toMatchObject({ enabled: false })
    expect(shellGate(rated({ shell: 'admin_only' }), 'admin')).toEqual({ enabled: true, reason: null })
    expect(shellGate(rated({ shell: 'per_role', level: 'ok', reasons: [] }), 'user')).toEqual({ enabled: true, reason: null })
    expect(shellGate(undefined, 'user')).toEqual({ enabled: true, reason: null })
    expect(shellGate(rated({ shell: 'admin_only' }), 'user').reason).toMatch(/administrator/)
  })

  it('tells only an admin, and only of a rootful daemon', () => {
    expect(rootfulNotice('admin', { ...features, daemon: 'rootful' })).toBe(true)
    expect(rootfulNotice('admin', { ...features, daemon: 'rootless' })).toBe(false)
    expect(rootfulNotice('admin', { ...features, daemon: 'userns' })).toBe(false)
    expect(rootfulNotice('admin', features)).toBe(false)
    expect(rootfulNotice('user', { ...features, daemon: 'rootful' })).toBe(false)
  })

  it('rates compose findings by their worst', () => {
    expect(worstFinding([warning('info'), warning('root'), warning('escape')])).toBe('escape')
    expect(worstFinding([warning('info'), warning('root')])).toBe('root')
    expect(worstFinding([warning('info')])).toBe('ok')
    expect(worstFinding(undefined)).toBe('ok')
  })
})

describe('051 SecurityChip', () => {
  it('renders nothing for an ok container', () => {
    const { container } = render(<SecurityChip level="ok" security={rated({ level: 'ok' })} />)
    expect(container).toBeEmptyDOMElement()
  })

  it('describes itself with the reasons and opens them on a tap', () => {
    render(<SecurityChip level="root" security={rated()} />)
    const chip = screen.getByRole('button', { name: 'Security: root' })
    expect(chip).toHaveAttribute('data-field', 'security')
    expect(chip).toHaveAttribute('data-level', 'root')
    expect(chip).toHaveAccessibleDescription('Runs as root (uid 0) on a rootful daemon.')
    const tip = screen.getByRole('tooltip', { hidden: true })
    expect(tip).toHaveClass('hidden')
    fireEvent.click(chip)
    expect(chip).toHaveAttribute('aria-expanded', 'true')
    expect(tip).toHaveClass('flex')
    fireEvent.keyDown(chip, { key: 'Escape' })
    expect(tip).toHaveClass('hidden')
  })

  it('says escape in red with reasons given directly', () => {
    render(<SecurityChip level="escape" reasons={['web-app-1 can reach the host.']} />)
    const chip = screen.getByRole('button', { name: 'Security: escape' })
    expect(chip.className).toContain('text-status-err')
    expect(chip).toHaveAccessibleDescription('web-app-1 can reach the host.')
  })
})
