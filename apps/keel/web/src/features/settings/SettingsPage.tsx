/**
 * Settings (spec 060, spec 066 R10), at `/settings/:tab`. Owners and admins see Workspace,
 * Enrollment, Tokens, Identity and System; the System tab is shaped by role (spec 061 FR-024). A
 * `ws_admin` of the current workspace sees Workspace alone. An unknown or forbidden tab is replaced
 * by Workspace; anyone else is told the page is for admins (the server refuses its reads anyway).
 *
 * Changes go through the governance confirm dialog; a Roll from Tokens opens the Enroll dialog's
 * result step, so both are mounted here.
 */
import { CertificateIcon, GearSixIcon, IdentificationCardIcon, KeyIcon, ShieldStarIcon, StackIcon } from '@phosphor-icons/react'
import { useEffect, type ReactNode } from 'react'
import { EmptyState, navigate, Page, PageHeader, Tabs, type Params } from '@companion/ui/kit'

import { useSession } from '@/app/session-store'
import { useCurrentWorkspace } from '@/app/workspace-store'
import { EnrollDialog } from '@/features/hosts/EnrollDialog'

import { EnrollmentTab } from './EnrollmentTab'
import { IdentityTab } from './IdentityTab'
import { SystemTab } from './SystemTab'
import { resolveTab, settingsPath, TAB_LABEL, visibleTabs, type SettingsTab } from './tabs'
import { TokensTab } from './TokensTab'
import { WorkspaceTab } from './WorkspaceTab'

const ICON: Record<SettingsTab, ReactNode> = {
  workspace: <StackIcon aria-hidden />,
  enrollment: <CertificateIcon aria-hidden />,
  tokens: <KeyIcon aria-hidden />,
  identity: <IdentificationCardIcon aria-hidden />,
  system: <ShieldStarIcon aria-hidden />,
}

function Panel({ tab }: { tab: SettingsTab }) {
  switch (tab) {
    case 'workspace':
      return <WorkspaceTab />
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
  const me = useSession((s) => s.me)
  const role = me?.instance_role
  const workspace = useCurrentWorkspace()?.id ?? null
  const asked = params.tab
  const tab = resolveTab(asked, role, me, workspace)
  const tabs = visibleTabs(role, me, workspace)

  // An unknown, forbidden or missing tab is replaced, so the address says what is shown.
  useEffect(() => {
    if (tabs.length > 0 && asked !== tab) navigate(settingsPath(tab), { replace: true })
  }, [asked, tab, tabs.length])

  return (
    <Page name="settings" labelledBy="settings-title">
      <PageHeader
        id="settings-title"
        title="Settings"
        description="The current workspace's repository and forge, how hosts enroll, every token on the instance, and how roles come from your identity provider."
      />
      {tabs.length === 0 ? (
        <EmptyState icon={<GearSixIcon aria-hidden />} title="For admins">
          Settings are managed by the instance&apos;s owners and admins, and by each workspace&apos;s admins.
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
      <EnrollDialog />
    </Page>
  )
}
