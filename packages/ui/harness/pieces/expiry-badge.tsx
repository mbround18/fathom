import { ExpiryBadge } from '../../src/kit'
import type { Fixture } from '../fixture'

/** A fixed clock, so each badge always lands in the same tone. */
const NOW = Date.parse('2026-11-02T12:00:00Z')
const days = (n: number) => new Date(NOW + n * 86_400_000).toISOString()

const fixture: Fixture = {
  id: 'expiry-badge',
  states: {
    tones: () => (
      <ul className="flex flex-col gap-2">
        <li>
          <ExpiryBadge notAfter={days(200)} now={NOW} />
        </li>
        <li>
          <ExpiryBadge notAfter={days(21)} now={NOW} />
        </li>
        <li>
          <ExpiryBadge notAfter={days(1)} now={NOW} />
        </li>
        <li>
          <ExpiryBadge notAfter={days(-2)} now={NOW} />
        </li>
        <li>
          <ExpiryBadge notAfter="not a time" now={NOW} />
        </li>
      </ul>
    ),
  },
}

export default fixture
