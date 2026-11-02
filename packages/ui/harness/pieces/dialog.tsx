import { useState } from 'react'

import { Button } from '../../src/button'
import { Dialog, FIELD, Field } from '../../src/kit'
import type { Fixture } from '../fixture'

/** A trigger that opens a rename dialog; the result shows under it. */
function Rename({ description, size }: { description?: string; size?: 'sm' | 'md' | 'lg' }) {
  const [open, setOpen] = useState(false)
  const [name, setName] = useState('Blue widget')
  const [saved, setSaved] = useState('Blue widget')
  const close = () => {
    setOpen(false)
  }
  return (
    <div className="flex flex-col items-start gap-2">
      <Button
        type="button"
        variant="outline"
        onClick={() => {
          setName(saved)
          setOpen(true)
        }}
      >
        Rename widget
      </Button>
      <p data-saved className="text-sm text-muted-foreground">
        Name: {saved}
      </p>
      <Dialog
        open={open}
        onClose={close}
        title="Rename widget"
        {...(description === undefined ? {} : { description })}
        {...(size === undefined ? {} : { size })}
        data-rename-dialog
        footer={
          <>
            <Button type="button" variant="ghost" onClick={close}>
              Cancel
            </Button>
            <Button
              type="button"
              onClick={() => {
                setSaved(name)
                close()
              }}
            >
              Save
            </Button>
          </>
        }
      >
        <Field label="Name" htmlFor="widget-name">
          <input
            id="widget-name"
            className={FIELD}
            value={name}
            onChange={(e) => {
              setName(e.target.value)
            }}
          />
        </Field>
      </Dialog>
    </div>
  )
}

const fixture: Fixture = {
  id: 'dialog',
  states: {
    default: () => <Rename description="The new name shows everywhere the widget is listed." />,
    'no-description': () => <Rename />,
    small: () => <Rename size="sm" description="The new name shows everywhere the widget is listed." />,
  },
}

export default fixture
