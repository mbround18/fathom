/**
 * Repository (spec 066 T051, contracts/ui.md "Repository page"): what the current workspace's
 * primary repository holds, from its catalog. Five tabs (Playbooks, Roles, Inventory, Secrets,
 * Jobs), each loaded when first opened; the tab is `?tab=` and the selected item `?item=`, so a
 * selection is a link. With no primary repository, or before its first sync, the page says so and
 * who can fix it (spec 063 empty states). A stale catalog says its last build failed.
 */
import { BookOpenIcon, FileCodeIcon, KeyIcon, ListChecksIcon, PlusIcon, StackIcon, TreeStructureIcon } from '@phosphor-icons/react'
import { Button } from '@companion/ui'
import { EmptyState, Link, LoadFailed, LoadState, navigate, Page, PageHeader, Tabs, Time, useSearchParam } from '@companion/ui/kit'
import { lazy, Suspense, useEffect, type ReactNode } from 'react'

import { useSession } from '@/app/session-store'
import { NoWorkspace } from '@/app/WorkspacePicker'
import { useCurrentWorkspace, withWorkspace } from '@/app/workspace-store'
import { AddHostDialog } from '@/features/hosts/AddHostDialog'
import { useAddHost } from '@/features/hosts/add-host-store'
import { canAdminister, canOperate } from '@/features/hosts/workspaces'
import { useGovernance } from '@/features/teams/store'
import type { CatalogView, WorkspaceInfo } from '@/protocol/wire'

import { isTab, repositoryHref, type RepoTab } from './address'
import { Flag } from './parts'
import { refusalCode, useCatalog } from './store'

const PlaybooksTab = lazy(() => import('./PlaybooksTab'))
const RolesTab = lazy(() => import('./RolesTab'))
const InventoryTab = lazy(() => import('./InventoryTab'))
const SecretsTab = lazy(() => import('./SecretsTab'))
const JobsTab = lazy(() => import('./JobsTab'))

const ITEMS: { value: RepoTab; label: string; icon: ReactNode }[] = [
  { value: 'playbooks', label: 'Playbooks', icon: <FileCodeIcon aria-hidden /> },
  { value: 'roles', label: 'Roles', icon: <StackIcon aria-hidden /> },
  { value: 'inventory', label: 'Inventory', icon: <TreeStructureIcon aria-hidden /> },
  { value: 'secrets', label: 'Secrets', icon: <KeyIcon aria-hidden /> },
  { value: 'jobs', label: 'Jobs', icon: <ListChecksIcon aria-hidden /> },
]

function Panel({ tab, catalog, workspace }: { tab: RepoTab; catalog: CatalogView; workspace: string }) {
  switch (tab) {
    case 'playbooks':
      return <PlaybooksTab catalog={catalog} workspace={workspace} />
    case 'roles':
      return <RolesTab catalog={catalog} />
    case 'inventory':
      return <InventoryTab catalog={catalog} workspace={workspace} />
    case 'secrets':
      return <SecretsTab catalog={catalog} />
    case 'jobs':
      return <JobsTab workspace={workspace} />
  }
}

/** No primary repository: an admin is sent to set one up, anyone else told who can. */
function NoRepository({ workspace, admin }: { workspace: WorkspaceInfo; admin: boolean }) {
  return (
    <EmptyState
      page="repository"
      icon={<BookOpenIcon aria-hidden />}
      title={admin ? 'Set up the repository' : 'No repository yet'}
      reason="The workspace's playbooks, roles, inventory and secrets come from its primary repository."
      steps={admin ? ['Bind the Ansible repository in Settings → Workspace.', 'Make it the primary repository.', 'Wait for the first sync.'] : undefined}
      action={
        admin ? (
          <Button asChild size="sm">
            <Link to={withWorkspace('/settings/workspace', workspace.slug)}>Set up the repository</Link>
          </Button>
        ) : undefined
      }
      whoCan="A workspace admin"
      verb="set up the repository"
      guide={{ guide: 'bindings', anchor: 'bind-a-repository' }}
    />
  )
}

function NotSynced() {
  return (
    <EmptyState
      page="repository-not-synced"
      title="Not synced yet"
      reason="Keel builds the catalog when the primary repository first syncs. This page updates when it does."
      guide={{ guide: 'bindings', anchor: 'bind-a-repository' }}
    />
  )
}

/** The catalog's commit and age, and a warning when its last build failed. */
function CatalogLine({ catalog }: { catalog: CatalogView }) {
  return (
    <div className="flex min-w-0 basis-full flex-wrap items-center gap-2 text-xs text-muted-foreground">
      <span>
        Commit <code className="font-mono">{catalog.commit.slice(0, 12)}</code>, built <Time at={catalog.built_at} relative />
      </span>
      {catalog.stale && (
        <span data-catalog-stale className="flex min-w-0 flex-wrap items-center gap-1">
          <Flag>stale</Flag>
          <span className="break-words text-status-warn">The last build failed{catalog.error ? `: ${catalog.error}` : '.'} Showing the last good catalog.</span>
        </span>
      )}
      {catalog.problems.length > 0 && <Flag title={catalog.problems.map((p) => `${p.path}: ${p.message}`).join('\n')}>{catalog.problems.length} problems</Flag>}
      {catalog.skipped.files > 0 && (
        <Flag tone="muted" title={catalog.skipped.reason ?? undefined}>
          {catalog.skipped.files} files skipped
        </Flag>
      )}
    </div>
  )
}

export default function RepositoryPage() {
  const workspace = useCurrentWorkspace()
  const listed = useGovernance((s) => s.workspaces !== null)
  const me = useSession((s) => s.me)
  const catalog = useCatalog((s) => s.catalog)
  const failed = useCatalog((s) => s.failed)
  const load = useCatalog((s) => s.load)
  const showAddHost = useAddHost((s) => s.show)
  const asked = useSearchParam('tab')
  const tab: RepoTab = isTab(asked) ? asked : 'playbooks'
  const id = workspace?.id ?? null
  const primary = workspace?.primary_binding ?? null

  useEffect(() => {
    if (id !== null) load({ id, primary_binding: primary })
  }, [id, primary, load])

  const code = refusalCode(failed)
  const admin = id !== null && canAdminister(me, id)
  const operator = id !== null && canOperate(me, id)
  const retry = () => {
    if (id !== null) load({ id, primary_binding: primary })
  }

  let body: ReactNode
  if (workspace === null) body = listed ? <NoWorkspace /> : <LoadState loading label="Loading workspaces…" />
  else if (primary === null || code === 'no_primary_repository') body = <NoRepository workspace={workspace} admin={admin} />
  else if (code === 'no_catalog') body = <NotSynced />
  else if (failed) body = <LoadFailed what="the catalog" refusal={failed} onRetry={retry} />
  else if (catalog === undefined) body = <LoadState loading label="Loading the catalog…" />
  else {
    body = (
      <Tabs
        label="Repository"
        items={ITEMS}
        value={tab}
        onChange={(t) => {
          navigateTab(t)
        }}
      >
        <div data-testid={`repo-tab-${tab}`} className="min-w-0">
          <Suspense fallback={<LoadState loading label="Loading…" />}>
            <Panel tab={tab} catalog={catalog} workspace={workspace.id} />
          </Suspense>
        </div>
      </Tabs>
    )
  }

  return (
    <Page name="repository" labelledBy="repository-title">
      <PageHeader
        id="repository-title"
        title="Repository"
        description="The playbooks, roles, inventory and secrets in this workspace's primary repository."
        guide={{ guide: 'bindings', anchor: 'the-manifest' }}
        actions={
          operator &&
          catalog !== undefined && (
            <Button
              size="sm"
              variant="outline"
              onClick={() => {
                showAddHost()
              }}
            >
              <PlusIcon aria-hidden />
              Add host
            </Button>
          )
        }
      >
        {catalog && workspace && !failed && <CatalogLine catalog={catalog} />}
      </PageHeader>
      {body}
      <AddHostDialog />
    </Page>
  )
}

/** A tab change clears the selection, and replaces the history entry so Back leaves the page. */
function navigateTab(tab: RepoTab): void {
  navigate(repositoryHref({ tab, item: null }), { replace: true })
}
