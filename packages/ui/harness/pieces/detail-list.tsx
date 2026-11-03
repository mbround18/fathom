import { DetailList, DetailRow, ExpiryBadge, Time, Unknown } from '../../src/kit'
import type { Fixture } from '../fixture'

/** A fixed clock, so the relative times never drift between runs. */
const NOW = Date.parse('2026-11-02T12:00:00Z')

const fixture: Fixture = {
  id: 'detail-list',
  states: {
    single: () => (
      <div className="max-w-xl">
        <DetailList label="Widget">
          <DetailRow term="Name" data="name">
            blue-widget
          </DetailRow>
          <DetailRow term="Address" data="address">
            <span className="font-mono">widgets.example.test:7443</span>
          </DetailRow>
          <DetailRow term="Created">
            <Time at="2026-10-30T08:15:00Z" />
          </DetailRow>
          <DetailRow term="Last seen">
            <Time at="2026-11-02T11:55:00Z" relative now={NOW} />
          </DetailRow>
          <DetailRow term="Owner" data="owner">
            <Unknown />
          </DetailRow>
          <DetailRow term="Notes">
            A long value wraps inside its own column rather than pushing the page sideways at a phone width.
          </DetailRow>
        </DetailList>
      </div>
    ),
    columns: () => (
      <div className="max-w-3xl">
        <DetailList label="Certificate" columns={2}>
          <DetailRow term="Subject">CN=widgets.example.test</DetailRow>
          <DetailRow term="Issuer">CN=Example Widget CA</DetailRow>
          <DetailRow term="Not before">
            <Time at="2026-09-01T00:00:00Z" />
          </DetailRow>
          <DetailRow term="Not after" data="not-after">
            <ExpiryBadge notAfter="2026-11-14T12:00:00Z" now={NOW} />
          </DetailRow>
        </DetailList>
      </div>
    ),
  },
}

export default fixture
