/**
 * Add host (spec 066 T053, contracts/ui.md "Add host"). Three steps:
 *
 * 1. Name, groups (the catalog's YAML inventory groups) and profiles.
 * 2. Review: the governance preview (`PreviewHostAdd`), which shows the inventory diff, then
 *    Confirm. This dialog steps aside while the change dialog is up, so one dialog holds focus.
 * 3. Result: the enrolment token and snippet, and what became of the inventory change (a pull
 *    request, a patch to apply by hand, or the forge's refusal with the patch).
 */
import { Button, Input } from '@companion/ui'
import { CopyBlock, Dialog, Field, LABEL } from '@companion/ui/kit'
import { useEffect, useId, useMemo } from 'react'

import { useCurrentWorkspace } from '@/app/workspace-store'
import { profileOptions, yamlGroups } from '@/features/repository/catalog'
import { ChangeOutcomeView } from '@/features/repository/ChangeOutcomeView'
import { useCatalog } from '@/features/repository/store'
import { useGovernance } from '@/features/teams/store'

import { useAddHost, type HostAddedResult } from './add-host-store'
import { ProfileChips } from './EnrollDialog'
import { HOST_NAME } from './enroll-store'
import { useHosts } from './store'

const CHIP =
  'inline-flex min-h-7 items-center border border-border px-2 text-xs font-mono hover:bg-muted/40 aria-pressed:border-accent-neon aria-pressed:bg-muted aria-pressed:text-foreground focus-visible:ring-1 focus-visible:ring-accent-neon focus-visible:outline-none'

function GroupPicker({ options, value, onChange }: { options: string[]; value: string[]; onChange: (next: string[]) => void }) {
  if (options.length === 0) {
    return <p className="text-xs text-muted-foreground">The repository has no YAML inventory groups. The host is added under all.</p>
  }
  return (
    <div role="group" aria-label="Groups" className="flex min-w-0 flex-wrap gap-1">
      {options.map((g) => {
        const on = value.includes(g)
        return (
          <button
            key={g}
            type="button"
            className={CHIP}
            aria-pressed={on}
            data-group-chip={g}
            onClick={() => {
              onChange(on ? value.filter((x) => x !== g) : [...value, g])
            }}
          >
            {g}
          </button>
        )
      })}
    </div>
  )
}

function Form({ workspace, onCancel }: { workspace: string; onCancel: () => void }) {
  const { name, groups: chosen, profiles: picked } = useAddHost((s) => s.draft)
  const edit = useAddHost((s) => s.edit)
  const added = useAddHost((s) => s.added)
  const catalog = useCatalog((s) => s.catalog)
  const jobs = useCatalog((s) => s.jobs)
  const hosts = useHosts((s) => s.hosts)
  const propose = useGovernance((s) => s.propose)
  const groups = useMemo(() => (catalog ? yamlGroups(catalog) : []), [catalog])
  const profiles = useMemo(() => profileOptions(jobs ?? [], hosts.flatMap((h) => h.profiles)), [jobs, hosts])
  const nameId = useId()
  const profilesId = useId()
  const valid = HOST_NAME.test(name)

  const submit = () => {
    if (!valid) return
    propose(
      `Add host ${name}`,
      { type: 'PreviewHostAdd', data: { workspace, inventory_name: name, groups: chosen, profiles: picked, inventory: null } },
      (event) => {
        if (event.type === 'HostAdded') added(event.data satisfies HostAddedResult)
      },
    )
  }

  return (
    <form
      id="add-host-form"
      className="flex min-w-0 flex-col gap-3"
      onSubmit={(e) => {
        e.preventDefault()
        submit()
      }}
    >
      <Field label="Inventory name" htmlFor={nameId} hint={name === '' || valid ? 'As it will appear in the inventory.' : 'Letters, digits, ., - and _.'}>
        <Input
          id={nameId}
          value={name}
          autoComplete="off"
          aria-invalid={name !== '' && !valid}
          onChange={(e) => {
            edit({ name: e.target.value.trim() })
          }}
        />
      </Field>
      <div className="flex min-w-0 flex-col gap-1">
        <span className={LABEL}>Groups</span>
        <GroupPicker options={groups} value={chosen} onChange={(groups) => {
            edit({ groups })
          }} />
      </div>
      <div className="flex min-w-0 flex-col gap-1">
        <label htmlFor={profilesId} className={LABEL}>
          Profiles
        </label>
        <ProfileChips id={profilesId} value={picked} onChange={(profiles) => {
            edit({ profiles })
          }} suggestions={profiles} />
      </div>
      <div className="flex justify-end gap-2">
        <Button type="button" variant="ghost" onClick={onCancel}>
          Cancel
        </Button>
        <Button type="submit" disabled={!valid}>
          Review
        </Button>
      </div>
    </form>
  )
}

function Result({ result }: { result: HostAddedResult }) {
  const { enrollment, change } = result
  return (
    <div data-add-host-result className="flex min-w-0 flex-col gap-3">
      <p className="text-xs">
        Run this on <span className="font-mono">{enrollment.inventory_name}</span> to enroll it. The token is shown once.
      </p>
      <CopyBlock label="Enrollment command" value={enrollment.snippet} data="enroll-snippet" />
      <div className="flex min-w-0 flex-col gap-1">
        <h3 className={LABEL}>Inventory change</h3>
        <ChangeOutcomeView outcome={change} />
      </div>
    </div>
  )
}

export function AddHostDialog() {
  const open = useAddHost((s) => s.open)
  const result = useAddHost((s) => s.result)
  const close = useAddHost((s) => s.close)
  const changing = useGovernance((s) => s.change !== null)
  const workspace = useCurrentWorkspace()
  const loadCatalog = useCatalog((s) => s.load)
  const catalogFor = useCatalog((s) => s.workspace)

  // The groups come from the catalog; Hosts may not have loaded it yet.
  useEffect(() => {
    if (open && workspace && catalogFor !== workspace.id) loadCatalog(workspace)
  }, [open, workspace, catalogFor, loadCatalog])

  return (
    <Dialog
      open={open && !changing && workspace !== null}
      onClose={close}
      size="md"
      data-add-host-dialog={result ? 'result' : 'form'}
      title={result ? 'Host added' : 'Add host'}
      description={
        result
          ? 'Keel issued an enrollment token and proposed the inventory change.'
          : 'Adds the host to the repository inventory and issues its enrollment token.'
      }
      footer={result ? <Button onClick={close}>Done</Button> : undefined}
    >
      {workspace && (result ? <Result result={result} /> : <Form workspace={workspace.id} onCancel={close} />)}
    </Dialog>
  )
}
