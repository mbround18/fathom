import { render, screen } from '@testing-library/react'
import { beforeEach, describe, expect, it } from 'vitest'

import type { CertManagerOverview } from '@/protocol/types'
import { useStore } from '@/store'

import { CertManagerPanel } from '@/components/panels/CertManagerPanel'

function setOverview(overview: CertManagerOverview | undefined) {
  useStore.setState({ certManager: { overview }, send: () => undefined } as never)
  useStore.setState({ refreshCertManager: () => undefined } as never)
}

describe('CertManagerPanel', () => {
  beforeEach(() => setOverview(undefined))

  it('renders the waiting state when there is no overview', () => {
    render(<CertManagerPanel />)
    expect(screen.getByText(/Waiting for the cert-manager scan/)).toBeTruthy()
  })
  it('renders the not-installed copy', () => {
    setOverview({ installed: false, certificates: [] })
    render(<CertManagerPanel />)
    expect(screen.getByText(/cert-manager is not installed/)).toBeTruthy()
  })
  it('renders the distinct zero-certificates copy', () => {
    setOverview({ installed: true, certificates: [] })
    render(<CertManagerPanel />)
    expect(screen.getByText(/no Certificates were found/)).toBeTruthy()
  })
})
