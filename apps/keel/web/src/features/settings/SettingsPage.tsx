/**
 * System Settings (spec 060), at `/settings/:tab`. Owners and admins see Enrollment, Tokens,
 * Identity and System; the System tab is shaped by role (spec 061 FR-024). An unknown or forbidden tab is replaced by Enrollment; anyone
 * else is told the page is for owners and admins (the server refuses its reads anyway).
 *
 * Changes go through the governance confirm dialog; a Roll from Tokens opens the Enroll dialog's
 * result step, so both are mounted here.
 */
import { CertificateIcon, GearSixIcon, IdentificationCardIcon, KeyIcon, ShieldStarIcon } from '@phosphor-icons/react'
import { useEffect, type ReactNode } from 'react'
import { EmptyState, navigate, Page, PageHeader, Tabs, type Params } from '@companion/ui/kit'

import { useSession } from '@/app/session-store'
import { EnrollDialog } from '@/features/hosts/EnrollDialog'
import { ChangeDialog } from '@/features/teams/ChangeDialog'

import { EnrollmentTab } from './EnrollmentTab'
import { IdentityTab } from './IdentityTab'
import { SystemTab } from './SystemTab'
import { resolveTab, settingsPath, TAB_LABEL, visibleTabs, type SettingsTab } from './tabs'
import { TokensTab } from './TokensTab'

const ICON: Record<SettingsTab, ReactNode> = {
  enrollment: <CertificateIcon aria-hidden />,
  tokens: <KeyIcon aria-hidden />,
  identity: <IdentificationCardIcon aria-hidden />,
  system: <ShieldStarIcon aria-hidden />,
}

function Panel({ tab }: { tab: SettingsTab }) {
  switch (tab) {
    case 'enrollment':
      return <EnrollmentTab />
    case 'tokens':
      return <TokensTab />
    case 'identity':
      return <IdentityTab />
    case 'system':
      return <SystemTab />
  }
}

export default function SettingsPage({ params }: { params: Params }) {
  const role = useSession((s) => s.me?.instance_role)
  const asked = params.tab
  const tab = resolveTab(asked, role)
  const tabs = visibleTabs(role)

  // An unknown, forbidden or missing tab is replaced, so the address says what is shown.
  useEffect(() => {
    if (tabs.length > 0 && asked !== tab) navigate(settingsPath(tab), { replace: true })
  }, [asked, tab, tabs.length])

  return (
    <Page name="settings" labelledBy="settings-title">
      <PageHeader
        id="settings-title"
        title="System Settings"
        description="How hosts enroll, every token on the instance, and how roles come from your identity provider."
      />
      {tabs.length === 0 ? (
        <EmptyState icon={<GearSixIcon aria-hidden />} title="For owners and admins">
          System Settings are managed by the instance&apos;s owners and admins.
        </EmptyState>
      ) : (
        <Tabs
          label="Settings"
          items={tabs.map((t) => ({ value: t, label: TAB_LABEL[t], icon: ICON[t] }))}
          value={tab}
          onChange={(t) => {
            navigate(settingsPath(t))
          }}
        >
          <Panel tab={tab} />
        </Tabs>
      )}
      <ChangeDialog />
      <EnrollDialog />
    </Page>
  )
}
