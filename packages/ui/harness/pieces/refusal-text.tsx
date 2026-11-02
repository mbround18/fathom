import { RefusalText, type Refusal } from '../../src/kit'
import type { Fixture } from '../fixture'

const DENIED: Refusal = { kind: 'denied', action: 'delete_widget', reason: 'no_grant' }
const ERROR: Refusal = { kind: 'error', code: 'conflict', message: 'The widget changed since you opened it.' }
/** The kit's offline shape; each app words its own. */
const OFFLINE: Refusal = { kind: 'error', code: 'offline', message: 'Not connected. Changes wait until the connection is back.' }

const fixture: Fixture = {
  id: 'refusal-text',
  states: {
    denied: () => <RefusalText refusal={DENIED} />,
    error: () => <RefusalText refusal={ERROR} />,
    offline: () => <RefusalText refusal={OFFLINE} />,
    lead: () => <RefusalText refusal={ERROR} lead="Could not save the widget." />,
  },
}

export default fixture
