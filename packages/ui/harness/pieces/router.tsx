import { useEffect, useState } from 'react'

import { Link, matchPath, setNavigationGuard, usePath } from '../../src/kit'
import type { Fixture } from '../fixture'

const LINK = 'text-primary underline underline-offset-2 focus-visible:ring-1 focus-visible:ring-accent-neon focus-visible:outline-none'

/** Where the router says we are, and which widget the path names. */
function Where() {
  const path = usePath()
  const id = matchPath('/widgets/:id', path)?.id
  return (
    <div className="flex flex-col gap-1 text-sm">
      <p>
        Path: <code data-path>{path}</code>
      </p>
      <p data-widget>{id === undefined ? 'No widget open.' : `Showing ${id} widget.`}</p>
    </div>
  )
}

function Links() {
  return (
    <nav aria-label="Widgets" className="flex flex-wrap gap-4 text-sm">
      <Link to="/widgets" className={LINK}>
        All widgets
      </Link>
      <Link to="/widgets/blue" className={LINK}>
        Blue widget
      </Link>
    </nav>
  )
}

/** A page with unsaved work: while the box is ticked, the guard keeps the router where it is. */
function Guarded() {
  const [dirty, setDirty] = useState(false)
  useEffect(() => {
    setNavigationGuard(() => !dirty)
    return () => {
      setNavigationGuard(null)
    }
  }, [dirty])
  return (
    <div className="flex flex-col gap-3">
      <label className="flex items-center gap-2 text-sm">
        <input
          type="checkbox"
          checked={dirty}
          onChange={(e) => {
            setDirty(e.target.checked)
          }}
        />
        Unsaved changes
      </label>
      <Links />
      <Where />
    </div>
  )
}

const fixture: Fixture = {
  id: 'router',
  states: {
    links: () => (
      <div className="flex flex-col gap-3">
        <Links />
        <Where />
      </div>
    ),
    guarded: () => <Guarded />,
  },
}

export default fixture
