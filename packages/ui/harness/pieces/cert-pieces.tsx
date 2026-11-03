import { useState } from 'react'

import { ChipList, DetailList, DetailRow, ExpiryBadge, Fingerprint, FORM_WIDTH, PemField, StageList } from '../../src/kit'
import type { Fixture } from '../fixture'

const NOW = Date.parse('2026-11-02T12:00:00Z')
const NAMES = ['widgets.example.test', 'www.widgets.example.test', 'api.widgets.example.test', 'eu.widgets.example.test', 'us.widgets.example.test', 'ap.widgets.example.test', 'staging.widgets.example.test', 'gadgets.example.test']

function Pem() {
  const [value, setValue] = useState('')
  return (
    <div className={FORM_WIDTH}>
      <PemField id="pem" label="Certificate chain" value={value} onChange={setValue} expect={['CERTIFICATE']} hint="The leaf first, then each intermediate." />
    </div>
  )
}

function Stages() {
  const [opened, setOpened] = useState('')
  return (
    <div className="flex max-w-xl flex-col gap-2">
      <StageList
        label="Issuance of widgets-tls"
        stages={[
          { key: 'certificate', title: 'Certificate widgets-tls', status: 'Not ready' },
          { key: 'request', title: 'CertificateRequest widgets-tls-1', status: 'Approved', state: 'done' },
          { key: 'order', title: 'Order widgets-tls-1-2093', status: 'Pending', state: 'current' },
          {
            key: 'challenge',
            title: 'Challenge widgets-tls-1-2093-0',
            status: 'Pending',
            state: 'stuck',
            note: 'Waiting for DNS-01 propagation: no TXT record at _acme-challenge.widgets.example.test',
            onOpen: () => {
              setOpened('challenge')
            },
          },
          { key: 'issued', title: 'Secret widgets-tls', state: 'pending' },
        ]}
      />
      <p data-opened={opened} className="text-xs text-muted-foreground">
        {opened ? `Opened ${opened}` : ''}
      </p>
    </div>
  )
}

const fixture: Fixture = {
  id: 'cert-pieces',
  states: {
    summary: () => (
      <div className="max-w-3xl">
        <DetailList label="widgets-tls" columns={2}>
          <DetailRow term="Subject">CN=widgets.example.test</DetailRow>
          <DetailRow term="Issuer">CN=Example Widget CA R3</DetailRow>
          <DetailRow term="Not after" data="expires">
            <ExpiryBadge notAfter="2026-11-23T12:00:00Z" now={NOW} />
          </DetailRow>
          <DetailRow term="Fingerprint">
            <Fingerprint value="sha256:3b4c5d6e7f8a9b0c1d2e3f4a5b6c7d8e9f0a1b2c3d4e5f6a7b8c9d0e1f2a3b4c" />
          </DetailRow>
          <DetailRow term="Names" data="names">
            <ChipList label="Subject alternative names" items={NAMES} max={3} />
          </DetailRow>
          <DetailRow term="Usages">
            <ChipList label="Key usages" items={['digital signature', 'key encipherment', 'server auth']} />
          </DetailRow>
        </DetailList>
      </div>
    ),
    pem: () => <Pem />,
    stages: () => <Stages />,
  },
}

export default fixture
