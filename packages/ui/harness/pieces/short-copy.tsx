import { DetailList, DetailRow, Fingerprint, ShortCopy } from '../../src/kit'
import type { Fixture } from '../fixture'

const fixture: Fixture = {
  id: 'short-copy',
  states: {
    ids: () => (
      <div className="max-w-xl">
        <DetailList label="Widget build">
          <DetailRow term="Digest">
            <ShortCopy label="digest" value="sha256:9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08" />
          </DetailRow>
          <DetailRow term="Revision">
            <ShortCopy label="revision" value="0a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6e7f8a9b" short="0a1b2c3" />
          </DetailRow>
          <DetailRow term="Key">
            <Fingerprint value="SHA256:uNiVztksCsDhcc0u9e8BujQXVUpKZIDTMczCvj3tD2s" />
          </DetailRow>
        </DetailList>
      </div>
    ),
  },
}

export default fixture
