/**
 * The workspace switcher in the shell's top bar (spec 066 T031, contracts/ui.md "Shell"). It
 * shows the current workspace and lists every visible one in slug order. Owners and admins also
 * get "Create workspace…", which previews and confirms through the governance `ChangeDialog`; the
 * new workspace becomes current once the list holds it.
 *
 * It mounts `useWorkspaceSync`, so the address and the stored choice follow the current workspace
 * on every page.
 */
import { CaretDownIcon, CheckIcon, PlusIcon, StackIcon } from '@phosphor-icons/react'
import {
  Button,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
  Input,
} from '@companion/ui'
import { Dialog, Field, LoadFailed } from '@companion/ui/kit'
import { useId, useState } from 'react'

import { canSeeSettings } from '@/features/settings/tabs'
import { useGovernance } from '@/features/teams/store'

import { useSession } from './session-store'
import {
  SLUG_PATTERN,
  slugFromName,
  useCurrentWorkspace,
  useWorkspaceStore,
  useWorkspaceSync,
  visibleWorkspaces,
} from './workspace-store'
import { NoWorkspace } from './WorkspacePicker'

function CreateWorkspaceDialog({ open, onClose }: { open: boolean; onClose: () => void }) {
  const propose = useGovernance((s) => s.propose)
  const chooseWhenListed = useWorkspaceStore((s) => s.chooseWhenListed)
  const [name, setName] = useState('')
  const [edited, setEdited] = useState<string | null>(null)
  const nameId = useId()
  const slugId = useId()
  const slug = edited ?? slugFromName(name)
  const slugOk = SLUG_PATTERN.test(slug)
  const ready = name.trim() !== '' && slugOk

  const close = () => {
    setName('')
    setEdited(null)
    onClose()
  }

  const submit = () => {
    if (!ready) return
    propose('New workspace', { type: 'PreviewWorkspaceCreate', data: { slug, name: name.trim() } }, (event) => {
      if (event.type === 'Confirmed') chooseWhenListed(slug)
    })
    close()
  }

  return (
    <Dialog
      open={open}
      onClose={close}
      data-create-workspace
      title="Create workspace"
      description="A workspace holds hosts, bindings and runs. You confirm the change on the next step."
      footer={
        <>
          <Button type="button" variant="ghost" onClick={close}>
            Cancel
          </Button>
          <Button type="submit" form={`${nameId}-form`} disabled={!ready}>
            Preview
          </Button>
        </>
      }
    >
      <form
        id={`${nameId}-form`}
        aria-label="Create workspace"
        className="flex flex-col gap-3"
        onSubmit={(e) => {
          e.preventDefault()
          submit()
        }}
      >
        <Field label="Name" htmlFor={nameId}>
          <Input
            id={nameId}
            value={name}
            maxLength={64}
            autoComplete="off"
            onChange={(e) => {
              setName(e.target.value)
            }}
          />
        </Field>
        <Field
          label="Slug"
          htmlFor={slugId}
          hint={slug !== '' && !slugOk ? 'Lowercase letters, digits and dashes, starting with a letter or digit.' : 'Used in links: ?ws=' + (slug || '…')}
        >
          <Input
            id={slugId}
            value={slug}
            maxLength={63}
            autoComplete="off"
            aria-invalid={slug !== '' && !slugOk}
            onChange={(e) => {
              setEdited(e.target.value)
            }}
          />
        </Field>
      </form>
    </Dialog>
  )
}

export function WorkspaceSwitcher() {
  useWorkspaceSync()
  const items = useGovernance((s) => s.workspaces)
  const refusal = useGovernance((s) => s.refused.workspaces)
  const load = useGovernance((s) => s.load)
  const role = useSession((s) => s.me?.instance_role)
  const choose = useWorkspaceStore((s) => s.choose)
  const current = useCurrentWorkspace()
  const [creating, setCreating] = useState(false)
  const all = items === null ? null : visibleWorkspaces(items)
  const canCreate = canSeeSettings(role)

  return (
    <>
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <Button
            variant="ghost"
            size="sm"
            data-testid="workspace-switcher"
            data-workspace={current?.slug ?? ''}
            aria-label={current ? `Workspace ${current.name}` : 'Workspace'}
            className="min-w-0 max-w-56"
          >
            <StackIcon aria-hidden />
            {current ? (
              <>
                <span className="hidden truncate sm:inline">{current.name}</span>
                <span className="truncate text-muted-foreground sm:max-w-20">{current.slug}</span>
              </>
            ) : (
              <span className="truncate text-muted-foreground">{all === null && !refusal ? 'Loading…' : 'No workspace'}</span>
            )}
            <CaretDownIcon aria-hidden />
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end" className="max-w-[calc(100vw-2rem)] min-w-56">
          {refusal && (
            <div className="p-2">
              <LoadFailed
                what="workspaces"
                refusal={refusal}
                onRetry={() => {
                  load('workspaces')
                }}
              />
            </div>
          )}
          {all?.length === 0 && (
            <div className="max-w-80 p-2">
              <NoWorkspace />
            </div>
          )}
          {all?.map((w) => (
            <DropdownMenuItem
              key={w.id}
              data-workspace-option={w.slug}
              aria-current={w.id === current?.id ? 'true' : undefined}
              onSelect={() => {
                choose(w.slug)
              }}
            >
              <CheckIcon aria-hidden className={`size-3.5 ${w.id === current?.id ? '' : 'invisible'}`} />
              <span className="min-w-0 truncate">{w.name}</span>
              <span className="ml-auto truncate pl-2 text-xs text-muted-foreground">
                {w.system ? `${w.slug} (System)` : w.slug}
              </span>
            </DropdownMenuItem>
          ))}
          {canCreate && (
            <>
              {(all?.length ?? 0) > 0 && <DropdownMenuSeparator />}
              <DropdownMenuItem
                data-action="create-workspace"
                onSelect={() => {
                  setCreating(true)
                }}
              >
                <PlusIcon aria-hidden className="size-3.5" />
                Create workspace…
              </DropdownMenuItem>
            </>
          )}
        </DropdownMenuContent>
      </DropdownMenu>
      <CreateWorkspaceDialog
        open={creating}
        onClose={() => {
          setCreating(false)
        }}
      />
    </>
  )
}
