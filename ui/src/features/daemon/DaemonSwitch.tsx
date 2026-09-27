/**
 * 043 US4: the in-cluster daemon preference switch, shared by the daemon dialog and
 * Settings › Cluster settings. Its name says what it controls and its description says what on
 * and off do (feedback #12: "I don't know what the on/off switch is for").
 */
import { useId } from 'react'

import { Switch } from '@/components/ui/switch'
import { useStore } from '@/store'

import {
  DAEMON_SWITCH_DESCRIPTION,
  DAEMON_SWITCH_LABEL,
  daemonState,
  useActiveDaemon,
} from './state'

export function DaemonSwitch({ testId = 'daemon-toggle' }: { testId?: string }) {
  const id = useId()
  const { context, daemon } = useActiveDaemon()
  const setPreference = useStore((s) => s.setDaemonPreference)
  if (!context) return null
  const enabled = daemonState(daemon) !== 'off'

  return (
    <div className="flex flex-col gap-1" data-testid={`${testId}-row`}>
      <div className="flex min-h-[var(--touch-target)] items-center justify-between gap-3 text-sm">
        <label htmlFor={`${id}-switch`} id={`${id}-label`}>
          {DAEMON_SWITCH_LABEL}
        </label>
        <Switch
          id={`${id}-switch`}
          checked={enabled}
          onCheckedChange={(on) => {
            setPreference(context, on ? 'auto' : 'off')
          }}
          data-testid={testId}
          aria-labelledby={`${id}-label`}
          aria-describedby={`${id}-description`}
        />
      </div>
      <p id={`${id}-description`} className="text-xs text-muted-foreground">
        {DAEMON_SWITCH_DESCRIPTION}
      </p>
    </div>
  )
}
