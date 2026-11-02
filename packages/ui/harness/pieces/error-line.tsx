import { useState } from 'react'

import { ErrorLine } from '../../src/kit'
import type { Fixture } from '../fixture'

const MESSAGE = 'The widget name is already taken.'

function Dismissable() {
  const [message, setMessage] = useState<string | null>(MESSAGE)
  return (
    <div className="flex flex-col gap-2">
      <ErrorLine
        message={message}
        onDismiss={() => {
          setMessage(null)
        }}
      />
      {message === null && (
        <p data-dismissed className="text-xs text-muted-foreground">
          No errors.
        </p>
      )}
    </div>
  )
}

const fixture: Fixture = {
  id: 'error-line',
  states: {
    dismiss: () => <Dismissable />,
    plain: () => <ErrorLine message={MESSAGE} />,
    multiline: () => <ErrorLine message={'Two widgets failed to save:\nBlue widget: the name is taken.\nGreen widget: the name is too long.'} />,
  },
}

export default fixture
