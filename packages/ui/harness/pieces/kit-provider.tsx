import { useKit } from '../../src/kit'
import type { Fixture } from '../fixture'

/** Shows what the harness's provider hands the pieces below it. */
function Context() {
  const kit = useKit()
  return (
    <dl className="grid grid-cols-[auto_1fr] gap-x-4 text-sm">
      <dt className="text-muted-foreground">Guide link</dt>
      <dd data-guide-href>{kit.guideHref?.({ guide: 'widgets', anchor: 'add-a-widget' }) ?? 'none'}</dd>
      <dt className="text-muted-foreground">Glossary terms</dt>
      <dd data-glossary-size>{Object.keys(kit.glossary ?? {}).length}</dd>
    </dl>
  )
}

const fixture: Fixture = {
  id: 'kit-provider',
  states: { default: () => <Context /> },
}

export default fixture
