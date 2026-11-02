import { CopyBlock } from '../../src/kit'
import type { Fixture } from '../fixture'

const fixture: Fixture = {
  id: 'copy-block',
  states: {
    copy: () => (
      <div className="flex max-w-xl flex-col gap-4">
        <CopyBlock label="Widget address" value="widgets.example.test:7443" data="address" />
        <CopyBlock
          label="Install command"
          value="curl -fsSL https://widgets.example.test/install.sh | sh -s -- --name blue-widget --join-token-file /etc/widgets/join-token --listen 0.0.0.0:7443"
          data="install"
        />
      </div>
    ),
    masked: () => (
      <div className="max-w-xl">
        <CopyBlock label="Join token" value="wjt_7f3a9c2e81b44d0f9a6e5c1d2b3a4f5e" masked data="token" />
      </div>
    ),
  },
}

export default fixture
