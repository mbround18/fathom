/**
 * Cluster Settings > Node Shell defaults (037-node-shell US4): image, namespace, pull secret and
 * auto-delete deadline, saved per context in `node-shell.json`. The backend re-validates and
 * replies with the normalized values (`NodeShellSettingsLoaded`); a session may still override
 * image and deadline from the Node Shell tab without changing these defaults.
 */
import { useEffect, useState } from 'react'

import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { useStore } from '@/store'

export function NodeShellSettingsSection() {
  const send = useStore((s) => s.send)
  const context = useStore((s) => s.session.activeContext)
  const loaded = useStore((s) => s.nodeShell.settings)
  const loadedContext = useStore((s) => s.nodeShell.settingsContext)

  const [image, setImage] = useState('')
  const [namespace, setNamespace] = useState('')
  const [pullSecret, setPullSecret] = useState('')
  const [minutes, setMinutes] = useState('')
  const [prefilledFor, setPrefilledFor] = useState<unknown>(undefined)

  useEffect(() => {
    send({ type: 'GetNodeShellSettings' })
  }, [send, context])

  // Prefill from each freshly loaded settings object (adjust-state-during-render pattern).
  if (loaded !== undefined && loadedContext === context && prefilledFor !== loaded) {
    setPrefilledFor(loaded)
    setImage(loaded.image)
    setNamespace(loaded.namespace)
    setPullSecret(loaded.image_pull_secret ?? '')
    setMinutes(String(Math.round(loaded.deadline_seconds / 60)))
  }

  if (loaded === undefined) return null

  const deadlineSeconds = Math.round(Number(minutes) * 60)
  const valid =
    image.trim() !== '' &&
    namespace.trim() !== '' &&
    Number.isFinite(deadlineSeconds) &&
    deadlineSeconds >= 300

  const save = () => {
    send({
      type: 'SetNodeShellSettings',
      data: {
        settings: {
          image: image.trim(),
          namespace: namespace.trim(),
          image_pull_secret: pullSecret.trim() === '' ? null : pullSecret.trim(),
          deadline_seconds: deadlineSeconds,
        },
      },
    })
  }

  return (
    <section
      aria-label="Node Shell"
      data-testid="node-shell-settings"
      className="flex flex-col gap-2 p-3"
    >
      <h3 className="font-display text-xs tracking-wider uppercase">Node Shell</h3>
      <div className="grid gap-2 text-xs sm:grid-cols-2">
        <div className="flex flex-col gap-1">
          <label htmlFor="ns-settings-image">Debug image</label>
          <Input
            id="ns-settings-image"
            data-testid="node-shell-settings-image"
            value={image}
            onChange={(e) => setImage(e.target.value)}
            className="font-mono"
          />
        </div>
        <div className="flex flex-col gap-1">
          <label htmlFor="ns-settings-namespace">Namespace</label>
          <Input
            id="ns-settings-namespace"
            data-testid="node-shell-settings-namespace"
            value={namespace}
            onChange={(e) => setNamespace(e.target.value)}
            className="font-mono"
          />
        </div>
        <div className="flex flex-col gap-1">
          <label htmlFor="ns-settings-secret">Image pull secret (optional)</label>
          <Input
            id="ns-settings-secret"
            value={pullSecret}
            onChange={(e) => setPullSecret(e.target.value)}
            className="font-mono"
          />
        </div>
        <div className="flex flex-col gap-1">
          <label htmlFor="ns-settings-deadline">Auto-delete after (minutes)</label>
          <Input
            id="ns-settings-deadline"
            data-testid="node-shell-settings-deadline"
            inputMode="numeric"
            value={minutes}
            onChange={(e) => setMinutes(e.target.value)}
            className="font-mono"
          />
        </div>
      </div>
      {!valid && (
        <p role="alert" className="text-xs text-status-warn">
          Image and namespace are required, and the deadline must be at least 5 minutes.
        </p>
      )}
      <div>
        <Button
          type="button"
          size="sm"
          data-testid="node-shell-settings-save"
          disabled={!valid}
          onClick={save}
        >
          Save Node Shell settings
        </Button>
      </div>
    </section>
  )
}
