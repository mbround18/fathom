import { useState } from 'react'

import { FIELD, NoMatch } from '../../src/kit'
import type { Fixture } from '../fixture'

const WIDGETS = ['Blue widget', 'Green widget', 'Red widget']

/** A filtered list: the filter empties it, and Clear filter brings the rows back. */
function Filtered({ message }: { message?: string }) {
  const [query, setQuery] = useState('purple')
  const shown = WIDGETS.filter((w) => w.toLowerCase().includes(query.toLowerCase()))
  return (
    <div className="flex flex-col gap-2">
      <label className="flex flex-col gap-1 text-xs text-muted-foreground">
        Filter widgets
        <input
          className={FIELD}
          value={query}
          onChange={(e) => {
            setQuery(e.target.value)
          }}
        />
      </label>
      {shown.length === 0 ? (
        <NoMatch
          what="widgets"
          {...(message === undefined ? {} : { message })}
          onClear={() => {
            setQuery('')
          }}
        />
      ) : (
        <ul aria-label="Widgets" className="flex flex-col gap-1 text-sm">
          {shown.map((w) => (
            <li key={w}>{w}</li>
          ))}
        </ul>
      )}
    </div>
  )
}

const fixture: Fixture = {
  id: 'no-match',
  states: {
    default: () => <Filtered />,
    message: () => <Filtered message="No widgets match “purple”." />,
  },
}

export default fixture
