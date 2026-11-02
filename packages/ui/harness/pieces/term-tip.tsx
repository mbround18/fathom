import { H2, TermTip } from '../../src/kit'
import type { Fixture } from '../fixture'

const fixture: Fixture = {
  id: 'term-tip',
  states: {
    // The glossary entry has a guide, so the tip ends in Learn more.
    guided: () => (
      <section aria-labelledby="widgets-heading" className="flex flex-col gap-2">
        <div className="flex items-center gap-1.5">
          <h2 id="widgets-heading" className={H2}>
            Widgets
          </h2>
          <TermTip term="widget" />
        </div>
        <p className="text-sm text-muted-foreground">Three widgets, newest first.</p>
      </section>
    ),
    plain: () => (
      <section aria-labelledby="gadgets-heading" className="flex flex-col gap-2">
        <div className="flex items-center gap-1.5">
          <h2 id="gadgets-heading" className={H2}>
            Gadgets
          </h2>
          <TermTip term="gadget" />
        </div>
        <p className="text-sm text-muted-foreground">No gadgets yet.</p>
      </section>
    ),
    // A term the glossary lacks renders nothing.
    unknown: () => (
      <div className="flex items-center gap-1.5">
        <h2 className={H2}>Sprockets</h2>
        <TermTip term="sprocket" />
      </div>
    ),
  },
}

export default fixture
