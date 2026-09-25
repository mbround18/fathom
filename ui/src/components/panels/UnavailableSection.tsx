import { Empty } from '@/components/shell/PaneStates'

/**
 * Honest placeholder for a navigator section that has no backend command in this build
 * (distinct from "Not ported yet", which implies it is coming). Nothing here is faked.
 */
export function UnavailableSection({ label, reason }: { label: string; reason?: string }) {
  return (
    <div data-testid="unavailable-section">
      <Empty
        title={`${label} not available in this build`}
        detail={
          reason ??
          'The Fathom backend has no command for this resource type, so it cannot be listed or inspected here.'
        }
      />
    </div>
  )
}
