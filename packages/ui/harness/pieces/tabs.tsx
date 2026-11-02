import { ChartBarIcon, GearIcon, ListIcon } from '@phosphor-icons/react'
import { useState } from 'react'

import { Tabs, type TabItem } from '../../src/kit'
import type { Fixture } from '../fixture'

type Section = 'overview' | 'activity' | 'settings'

const ITEMS: readonly TabItem<Section>[] = [
  { value: 'overview', label: 'Overview', icon: <ChartBarIcon aria-hidden /> },
  { value: 'activity', label: 'Activity', icon: <ListIcon aria-hidden /> },
  { value: 'settings', label: 'Settings', icon: <GearIcon aria-hidden /> },
]

const MANY = [
  'Overview',
  'Activity',
  'Settings',
  'Members',
  'Schedules',
  'Notifications',
  'Integrations',
  'History',
].map((label) => ({ value: label.toLowerCase(), label }))

function Controlled<T extends string>({ items, first }: { items: readonly TabItem<T>[]; first: T }) {
  const [value, setValue] = useState<T>(first)
  const label = items.find((i) => i.value === value)?.label ?? ''
  return (
    <Tabs label="Widget sections" items={items} value={value} onChange={setValue}>
      <p className="text-sm text-muted-foreground">The {label.toLowerCase()} of Blue widget.</p>
    </Tabs>
  )
}

const fixture: Fixture = {
  id: 'tabs',
  states: {
    default: () => <Controlled items={ITEMS} first="overview" />,
    many: () => <Controlled items={MANY} first="overview" />,
  },
}

export default fixture
