import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { describe, expect, it, vi } from 'vitest'

import { FIELD, FIELD_DENSE, Field, FormPanel, KitProvider, SectionIntro, Select } from '../../src/kit'

describe('form pieces', () => {
  it('Field labels its control and shows the hint', () => {
    render(
      <Field label="Name" htmlFor="n" hint="Shown in every list.">
        <input id="n" />
      </Field>,
    )
    expect(screen.getByRole('textbox', { name: 'Name' })).toBeInTheDocument()
    expect(screen.getByText('Shown in every list.')).toBeInTheDocument()
  })

  it('Select takes the field look, or the dense one, plus the caller’s classes', () => {
    render(
      <>
        <Select aria-label="Full" className="w-40" />
        <Select aria-label="Dense" dense />
      </>,
    )
    expect(screen.getByRole('combobox', { name: 'Full' }).className).toBe(`${FIELD} w-40`)
    expect(screen.getByRole('combobox', { name: 'Dense' }).className.trim()).toBe(FIELD_DENSE)
  })

  it('FormPanel is a named form with its heading, and submits without reloading', async () => {
    const onSubmit = vi.fn()
    render(
      <FormPanel label="Add widget" onSubmit={onSubmit}>
        <button type="submit">Save</button>
      </FormPanel>,
    )
    expect(screen.getByRole('form', { name: 'Add widget' })).toBeInTheDocument()
    expect(screen.getByRole('heading', { level: 2, name: 'Add widget' })).toBeInTheDocument()
    await userEvent.click(screen.getByRole('button', { name: 'Save' }))
    expect(onSubmit).toHaveBeenCalledTimes(1)
  })

  it('SectionIntro links its guide when the app builds guide routes', () => {
    render(
      <KitProvider value={{ guideHref: (r) => `/help/${r.guide}#${r.anchor ?? ''}` }}>
        <SectionIntro guide={{ guide: 'widgets', anchor: 'sharing' }}>Who else can see it.</SectionIntro>
      </KitProvider>,
    )
    expect(screen.getByRole('link', { name: 'Learn more' })).toHaveAttribute('href', '/help/widgets#sharing')
  })
})
