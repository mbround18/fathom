import { render, screen } from '@testing-library/react'
import { expect, test } from 'vitest'

import { App } from '@/app/App'

test('the shell shows the Keel title', () => {
  render(<App />)
  expect(screen.getByRole('heading', { name: /Keel/ })).toBeInTheDocument()
})
