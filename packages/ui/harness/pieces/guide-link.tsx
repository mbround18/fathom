import { GuideLink, KitProvider } from '../../src/kit'
import type { Fixture } from '../fixture'

const fixture: Fixture = {
  id: 'guide-link',
  states: {
    default: () => (
      <p className="flex flex-wrap items-baseline gap-2 text-sm text-muted-foreground">
        Share a widget with the people who need it.
        <GuideLink to={{ guide: 'widgets', anchor: 'sharing-a-widget' }} />
      </p>
    ),
    label: () => (
      <p className="flex flex-wrap items-baseline gap-2 text-sm text-muted-foreground">
        New here?
        <GuideLink to={{ guide: 'widgets' }}>Read the widgets guide</GuideLink>
      </p>
    ),
    // An app without guides hands over no guideHref, and the link is left out.
    'no-guides': () => (
      <KitProvider value={{}}>
        <p className="flex flex-wrap items-baseline gap-2 text-sm text-muted-foreground">
          Share a widget with the people who need it.
          <GuideLink to={{ guide: 'widgets', anchor: 'sharing-a-widget' }} />
        </p>
      </KitProvider>
    ),
  },
}

export default fixture
