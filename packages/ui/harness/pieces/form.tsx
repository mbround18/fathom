import { useState } from 'react'

import { Button } from '../../src/button'
import { FIELD, Field, FormPanel, H2, LABEL, SectionIntro, Select } from '../../src/kit'
import type { Fixture } from '../fixture'

function Panel() {
  const [saved, setSaved] = useState<string | null>(null)
  const [name, setName] = useState('')
  const [kind, setKind] = useState('gauge')
  return (
    <div className="flex flex-col gap-2">
      <FormPanel
        label="Add widget"
        onSubmit={() => {
          setSaved(`Added ${name || 'an unnamed widget'} (${kind}).`)
        }}
      >
        <Field label="Name" htmlFor="widget-name" hint="Shown in every list.">
          <input
            id="widget-name"
            className={FIELD}
            value={name}
            onChange={(e) => {
              setName(e.target.value)
            }}
          />
        </Field>
        <Field label="Type" htmlFor="widget-type">
          <Select
            id="widget-type"
            value={kind}
            onChange={(e) => {
              setKind(e.target.value)
            }}
          >
            <option value="gauge">Gauge</option>
            <option value="counter">Counter</option>
            <option value="timer">Timer</option>
          </Select>
        </Field>
        <div className="flex justify-end">
          <Button type="submit">Add widget</Button>
        </div>
      </FormPanel>
      <p role="status" className="text-sm text-muted-foreground">
        {saved ?? ''}
      </p>
    </div>
  )
}

const fixture: Fixture = {
  id: 'form',
  states: {
    field: () => (
      <div className="flex max-w-sm flex-col gap-3">
        <Field label="Name" htmlFor="widget-name" hint="Letters, digits and dashes.">
          <input id="widget-name" className={FIELD} defaultValue="blue-widget" />
        </Field>
        <Field label="Owner" htmlFor="widget-owner">
          <input id="widget-owner" className={FIELD} defaultValue="" aria-invalid="true" aria-describedby="owner-error" />
        </Field>
        <p id="owner-error" className="text-xs text-status-err">
          An owner is required.
        </p>
      </div>
    ),
    label: () => (
      <dl className="grid max-w-sm grid-cols-[auto_1fr] gap-x-4 gap-y-1 text-sm">
        <dt className={LABEL}>Name</dt>
        <dd>Blue widget</dd>
        <dt className={LABEL}>Type</dt>
        <dd>Gauge</dd>
      </dl>
    ),
    select: () => (
      <div className="flex max-w-sm flex-col gap-3">
        <Field label="Type" htmlFor="widget-type">
          <Select id="widget-type" defaultValue="counter">
            <option value="gauge">Gauge</option>
            <option value="counter">Counter</option>
            <option value="timer">Timer</option>
          </Select>
        </Field>
        <Field label="Sort" htmlFor="widget-sort">
          <Select id="widget-sort" dense defaultValue="name">
            <option value="name">Name</option>
            <option value="newest">Newest</option>
          </Select>
        </Field>
      </div>
    ),
    panel: () => <Panel />,
    'section-intro': () => (
      <section aria-labelledby="sharing-title" className="flex flex-col gap-1">
        <h2 id="sharing-title" className={H2}>
          Sharing
        </h2>
        <SectionIntro guide={{ guide: 'widgets', anchor: 'sharing' }}>Who else can see this widget.</SectionIntro>
      </section>
    ),
  },
}

export default fixture
