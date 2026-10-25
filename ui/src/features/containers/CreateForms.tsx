/**
 * 048-docker-management US7 (T077): creating things, from fields and nothing else.
 *
 * Every form here collects the documented fields of one `ContainerSpec`, `ImageBuildSpec`,
 * `NetworkSpec` or `VolumeSpec`, and the backend turns those fields into an argv whose every
 * element came from exactly one field. There is deliberately no field for "extra arguments" and no
 * place to type a command line: FR-019 says every operation must be describable in advance, and a
 * string nobody can enumerate is a string nobody can preview (the protocol guards the same rule in
 * `container_runtime.rs::tests::no_free_text_argument_field`).
 *
 * That limit is stated on screen rather than merely enforced (FR-034). An operator who needs
 * something these fields cannot express is not being told "no" — they are being told that the
 * place for it is a terminal, where it is theirs and Fathom is not pretending to have previewed
 * it. A form that silently lacked the field would read as a missing feature; a form that says so
 * reads as a boundary.
 *
 * Nothing here runs anything. Every form ends at `previewContainerOperation`, so the operator's
 * last act is still approving the argv in `OperationPreview` — the same two-phase path every
 * other mutation in this feature takes.
 */
import { PlusIcon, XIcon } from '@phosphor-icons/react'
import { useId, useState } from 'react'

import { Button, Checkbox, Input } from '@companion/ui'
import type { ObjectClass, OperationRequest, PublishedPort } from '@/protocol/types'
import { useStore } from '@/store'

/** The sentence FR-034 asks for, in one place so every form says it the same way. */
export const TERMINAL_NOTE =
  'These are the fields Fathom can describe in advance. Anything this form does not cover belongs in a terminal: Fathom will not accept a command line it cannot show you first.'

function Field({
  label,
  children,
  hint,
}: {
  label: string
  children: React.ReactNode
  hint?: string
}) {
  return (
    <label className="flex min-w-0 flex-col gap-1 text-xs">
      <span className="text-muted-foreground">{label}</span>
      {children}
      {hint !== undefined && <span className="text-[10px] text-muted-foreground">{hint}</span>}
    </label>
  )
}

/**
 * A list of pairs the operator can grow and shrink.
 *
 * Two inputs per row rather than one `K=V` box, because the backend sends the pair as two halves
 * of one argv element and a value that happens to contain an `=` must not change which half it
 * lands in.
 */
function PairRows({
  testId,
  rows,
  onChange,
  keyLabel,
  valueLabel,
  addLabel,
}: {
  testId: string
  rows: [string, string][]
  onChange: (rows: [string, string][]) => void
  keyLabel: string
  valueLabel: string
  addLabel: string
}) {
  return (
    <div className="flex min-w-0 flex-col gap-1">
      {rows.map(([k, v], i) => (
        <div key={i} className="flex min-w-0 items-center gap-1">
          <Input
            aria-label={`${keyLabel} ${String(i + 1)}`}
            data-testid={`${testId}-key-${String(i)}`}
            value={k}
            autoComplete="off"
            onChange={(e) => {
              const next = [...rows]
              next[i] = [e.target.value, v]
              onChange(next)
            }}
          />
          <Input
            aria-label={`${valueLabel} ${String(i + 1)}`}
            data-testid={`${testId}-value-${String(i)}`}
            value={v}
            autoComplete="off"
            onChange={(e) => {
              const next = [...rows]
              next[i] = [k, e.target.value]
              onChange(next)
            }}
          />
          <Button
            size="xs"
            variant="ghost"
            aria-label={`Remove ${keyLabel.toLowerCase()} ${String(i + 1)}`}
            data-testid={`${testId}-remove-${String(i)}`}
            onClick={() => {
              onChange(rows.filter((_, j) => j !== i))
            }}
          >
            <XIcon aria-hidden="true" />
          </Button>
        </div>
      ))}
      <Button
        size="xs"
        variant="ghost"
        className="self-start"
        data-testid={`${testId}-add`}
        onClick={() => {
          onChange([...rows, ['', '']])
        }}
      >
        <PlusIcon data-icon="inline-start" /> {addLabel}
      </Button>
    </div>
  )
}

/** The pairs that are worth sending: a half-typed row is not a field the operator filled in. */
function filled(rows: [string, string][]): [string, string][] {
  return rows.filter(([k, v]) => k.trim() !== '' && v.trim() !== '')
}

function Footer({
  canPreview,
  onPreview,
  onCancel,
  testId,
}: {
  canPreview: boolean
  onPreview: () => void
  onCancel: () => void
  testId: string
}) {
  return (
    <>
      <p data-testid={`${testId}-terminal-note`} className="text-[10px] text-muted-foreground">
        {TERMINAL_NOTE}
      </p>
      <div className="flex shrink-0 gap-2">
        <Button
          size="sm"
          data-testid={`${testId}-preview`}
          disabled={!canPreview}
          onClick={onPreview}
        >
          Show me what this runs
        </Button>
        <Button size="sm" variant="ghost" data-testid={`${testId}-cancel`} onClick={onCancel}>
          Cancel
        </Button>
      </div>
    </>
  )
}

function Frame({ testId, children }: { testId: string; children: React.ReactNode }) {
  return (
    <div
      data-testid={testId}
      className="flex min-w-0 shrink-0 flex-col gap-3 border-t border-border bg-card p-3"
    >
      {children}
    </div>
  )
}

/**
 * `docker run` / `docker create`, field by field.
 *
 * The command is a list of words, one input each, not a line of shell. That is the whole
 * difference between an argv the preview can promise and a string the daemon's `/bin/sh` would
 * reinterpret: a word with a space, a quote or a semicolon in it stays one argument.
 */
export function CreateContainerForm({ onDone }: { onDone: () => void }) {
  const preview = useStore((s) => s.previewContainerOperation)
  const [image, setImage] = useState('')
  const [name, setName] = useState('')
  const [ports, setPorts] = useState<[string, string][]>([])
  const [env, setEnv] = useState<[string, string][]>([])
  const [volumes, setVolumes] = useState<[string, string][]>([])
  const [networks, setNetworks] = useState('')
  const [words, setWords] = useState<string[]>([])
  const [restart, setRestart] = useState('')
  const [detach, setDetach] = useState(true)
  const detachId = useId()

  const submit = () => {
    const published: PublishedPort[] = filled(ports).map(([host, container]) => ({
      container_port: Number(container),
      // An empty host side is a port exposed and not published: a host port Fathom chose would
      // bind something the operator never asked to bind.
      host_port: host.trim() === '' ? null : Number(host),
      host_ip: null,
      protocol: 'tcp',
    }))
    const request: OperationRequest = {
      kind: 'create_container',
      spec: {
        image: image.trim(),
        name: name.trim() === '' ? null : name.trim(),
        ports: published,
        env: filled(env),
        volumes: filled(volumes),
        networks: networks
          .split(',')
          .map((n) => n.trim())
          .filter((n) => n !== ''),
        command: words.map((w) => w.trim()).filter((w) => w !== ''),
        restart_policy: restart === '' ? null : restart,
        detach,
      },
    }
    preview(request)
    onDone()
  }

  return (
    <Frame testId="create-container-form">
      <Field label="Image">
        <Input
          data-testid="create-container-image"
          value={image}
          autoComplete="off"
          placeholder="busybox:latest"
          onChange={(e) => setImage(e.target.value)}
        />
      </Field>
      <Field label="Name" hint="Left empty, the runtime names it.">
        <Input
          data-testid="create-container-name"
          value={name}
          autoComplete="off"
          onChange={(e) => setName(e.target.value)}
        />
      </Field>
      <Field
        label="Ports"
        hint="Host port, then container port. An empty host port only exposes it."
      >
        <PairRows
          testId="create-container-port"
          rows={ports}
          onChange={setPorts}
          keyLabel="Host port"
          valueLabel="Container port"
          addLabel="Add a port"
        />
      </Field>
      <Field label="Environment">
        <PairRows
          testId="create-container-env"
          rows={env}
          onChange={setEnv}
          keyLabel="Variable"
          valueLabel="Value"
          addLabel="Add a variable"
        />
      </Field>
      <Field label="Volumes" hint="Source on this machine, then the path inside the container.">
        <PairRows
          testId="create-container-volume"
          rows={volumes}
          onChange={setVolumes}
          keyLabel="Source"
          valueLabel="Target"
          addLabel="Add a volume"
        />
      </Field>
      <Field label="Networks" hint="Comma separated.">
        <Input
          data-testid="create-container-networks"
          value={networks}
          autoComplete="off"
          onChange={(e) => setNetworks(e.target.value)}
        />
      </Field>
      <Field
        label="Command"
        hint="One word per box. This is not a shell line: each word is passed through as it is written."
      >
        <div className="flex min-w-0 flex-col gap-1">
          {words.map((w, i) => (
            <div key={i} className="flex min-w-0 items-center gap-1">
              <Input
                aria-label={`Command word ${String(i + 1)}`}
                data-testid={`create-container-word-${String(i)}`}
                value={w}
                autoComplete="off"
                onChange={(e) => {
                  const next = [...words]
                  next[i] = e.target.value
                  setWords(next)
                }}
              />
              <Button
                size="xs"
                variant="ghost"
                aria-label={`Remove command word ${String(i + 1)}`}
                data-testid={`create-container-word-remove-${String(i)}`}
                onClick={() => {
                  setWords(words.filter((_, j) => j !== i))
                }}
              >
                <XIcon aria-hidden="true" />
              </Button>
            </div>
          ))}
          <Button
            size="xs"
            variant="ghost"
            className="self-start"
            data-testid="create-container-word-add"
            onClick={() => {
              setWords([...words, ''])
            }}
          >
            <PlusIcon data-icon="inline-start" /> Add a word
          </Button>
        </div>
      </Field>
      <Field label="Restart policy">
        <select
          data-testid="create-container-restart"
          className="h-8 min-w-0 border border-input bg-transparent px-2 text-xs"
          value={restart}
          onChange={(e) => setRestart(e.target.value)}
        >
          <option value="">The runtime's default</option>
          <option value="no">no</option>
          <option value="on-failure">on-failure</option>
          <option value="unless-stopped">unless-stopped</option>
          <option value="always">always</option>
        </select>
      </Field>
      <div className="flex min-w-0 items-center gap-2 text-xs">
        <Checkbox
          id={detachId}
          data-testid="create-container-detach"
          checked={detach}
          onCheckedChange={(v) => setDetach(v === true)}
        />
        <label htmlFor={detachId}>Start it now (unchecked: create it and leave it stopped)</label>
      </div>
      <Footer
        testId="create-container"
        canPreview={image.trim() !== ''}
        onPreview={submit}
        onCancel={onDone}
      />
    </Frame>
  )
}

/** `docker build`. The context is a path on this machine; Fathom reads neither it nor the file. */
export function BuildImageForm({ onDone }: { onDone: () => void }) {
  const preview = useStore((s) => s.previewContainerOperation)
  const [tag, setTag] = useState('')
  const [context, setContext] = useState('')
  const [dockerfile, setDockerfile] = useState('')
  const [target, setTarget] = useState('')
  const [args, setArgs] = useState<[string, string][]>([])
  const [noCache, setNoCache] = useState(false)
  const cacheId = useId()

  const submit = () => {
    preview({
      kind: 'build_image',
      spec: {
        context_path: context.trim(),
        dockerfile: dockerfile.trim() === '' ? null : dockerfile.trim(),
        tag: tag.trim(),
        build_args: filled(args),
        target: target.trim() === '' ? null : target.trim(),
        no_cache: noCache,
      },
    })
    onDone()
  }

  return (
    <Frame testId="build-image-form">
      <Field label="Tag">
        <Input
          data-testid="build-image-tag"
          value={tag}
          autoComplete="off"
          placeholder="my-app:dev"
          onChange={(e) => setTag(e.target.value)}
        />
      </Field>
      <Field label="Build context" hint="A directory on this machine.">
        <Input
          data-testid="build-image-context"
          value={context}
          autoComplete="off"
          placeholder="/srv/my-app"
          onChange={(e) => setContext(e.target.value)}
        />
      </Field>
      <Field label="Dockerfile" hint="Left empty, the builder looks in the context.">
        <Input
          data-testid="build-image-dockerfile"
          value={dockerfile}
          autoComplete="off"
          onChange={(e) => setDockerfile(e.target.value)}
        />
      </Field>
      <Field label="Target stage">
        <Input
          data-testid="build-image-target"
          value={target}
          autoComplete="off"
          onChange={(e) => setTarget(e.target.value)}
        />
      </Field>
      <Field label="Build arguments">
        <PairRows
          testId="build-image-arg"
          rows={args}
          onChange={setArgs}
          keyLabel="Argument"
          valueLabel="Value"
          addLabel="Add an argument"
        />
      </Field>
      <div className="flex min-w-0 items-center gap-2 text-xs">
        <Checkbox
          id={cacheId}
          data-testid="build-image-no-cache"
          checked={noCache}
          onCheckedChange={(v) => setNoCache(v === true)}
        />
        <label htmlFor={cacheId}>Build every layer again, ignoring the cache</label>
      </div>
      <Footer
        testId="build-image"
        canPreview={tag.trim() !== '' && context.trim() !== ''}
        onPreview={submit}
        onCancel={onDone}
      />
    </Frame>
  )
}

/** `docker network create`. */
export function CreateNetworkForm({ onDone }: { onDone: () => void }) {
  const preview = useStore((s) => s.previewContainerOperation)
  const [name, setName] = useState('')
  const [driver, setDriver] = useState('')
  const [subnet, setSubnet] = useState('')
  const [gateway, setGateway] = useState('')
  const [internal, setInternal] = useState(false)
  const [labels, setLabels] = useState<[string, string][]>([])
  const internalId = useId()

  return (
    <Frame testId="create-network-form">
      <Field label="Name">
        <Input
          data-testid="create-network-name"
          value={name}
          autoComplete="off"
          onChange={(e) => setName(e.target.value)}
        />
      </Field>
      <Field label="Driver" hint="Left empty, the runtime's default.">
        <Input
          data-testid="create-network-driver"
          value={driver}
          autoComplete="off"
          placeholder="bridge"
          onChange={(e) => setDriver(e.target.value)}
        />
      </Field>
      <Field label="Subnet">
        <Input
          data-testid="create-network-subnet"
          value={subnet}
          autoComplete="off"
          placeholder="172.30.0.0/16"
          onChange={(e) => setSubnet(e.target.value)}
        />
      </Field>
      <Field label="Gateway">
        <Input
          data-testid="create-network-gateway"
          value={gateway}
          autoComplete="off"
          onChange={(e) => setGateway(e.target.value)}
        />
      </Field>
      <Field label="Labels">
        <PairRows
          testId="create-network-label"
          rows={labels}
          onChange={setLabels}
          keyLabel="Label"
          valueLabel="Value"
          addLabel="Add a label"
        />
      </Field>
      <div className="flex min-w-0 items-center gap-2 text-xs">
        <Checkbox
          id={internalId}
          data-testid="create-network-internal"
          checked={internal}
          onCheckedChange={(v) => setInternal(v === true)}
        />
        <label htmlFor={internalId}>Internal: no route out of this network</label>
      </div>
      <Footer
        testId="create-network"
        canPreview={name.trim() !== ''}
        onPreview={() => {
          preview({
            kind: 'create_network',
            spec: {
              name: name.trim(),
              driver: driver.trim() === '' ? null : driver.trim(),
              subnet: subnet.trim() === '' ? null : subnet.trim(),
              gateway: gateway.trim() === '' ? null : gateway.trim(),
              internal,
              labels: filled(labels),
            },
          })
          onDone()
        }}
        onCancel={onDone}
      />
    </Frame>
  )
}

/** `docker volume create`. */
export function CreateVolumeForm({ onDone }: { onDone: () => void }) {
  const preview = useStore((s) => s.previewContainerOperation)
  const [name, setName] = useState('')
  const [driver, setDriver] = useState('')
  const [labels, setLabels] = useState<[string, string][]>([])

  return (
    <Frame testId="create-volume-form">
      <Field label="Name">
        <Input
          data-testid="create-volume-name"
          value={name}
          autoComplete="off"
          onChange={(e) => setName(e.target.value)}
        />
      </Field>
      <Field label="Driver" hint="Left empty, the runtime's default.">
        <Input
          data-testid="create-volume-driver"
          value={driver}
          autoComplete="off"
          placeholder="local"
          onChange={(e) => setDriver(e.target.value)}
        />
      </Field>
      <Field label="Labels">
        <PairRows
          testId="create-volume-label"
          rows={labels}
          onChange={setLabels}
          keyLabel="Label"
          valueLabel="Value"
          addLabel="Add a label"
        />
      </Field>
      <Footer
        testId="create-volume"
        canPreview={name.trim() !== ''}
        onPreview={() => {
          preview({
            kind: 'create_volume',
            spec: {
              name: name.trim(),
              driver: driver.trim() === '' ? null : driver.trim(),
              labels: filled(labels),
            },
          })
          onDone()
        }}
        onCancel={onDone}
      />
    </Frame>
  )
}

/** The form for one object pane's own class, so `ObjectPane` does not have to know which. */
export function CreateObjectForm({
  objectClass,
  onDone,
}: {
  objectClass: Exclude<ObjectClass, 'container'>
  onDone: () => void
}) {
  if (objectClass === 'image') return <BuildImageForm onDone={onDone} />
  if (objectClass === 'network') return <CreateNetworkForm onDone={onDone} />
  return <CreateVolumeForm onDone={onDone} />
}

/** The word on the button that opens one pane's creation form. */
export function createLabel(objectClass: Exclude<ObjectClass, 'container'>): string {
  if (objectClass === 'image') return 'Build an image'
  if (objectClass === 'network') return 'Create a network'
  return 'Create a volume'
}
