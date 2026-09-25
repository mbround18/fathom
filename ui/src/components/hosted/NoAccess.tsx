import { HostedPanel } from './HostedPanel'

/** Zero-permission user (FR-007): shown instead of an empty, confusing UI. */
export function NoAccess({
  username,
  message,
  onSignOut,
}: {
  username: string
  /** Backend-provided refusal text, shown verbatim. */
  message?: string
  onSignOut?: () => void
}) {
  return (
    <HostedPanel
      title="No access"
      actions={
        onSignOut && (
          <button
            type="button"
            onClick={onSignOut}
            className="h-8 border border-border px-3 font-display text-xs font-semibold tracking-wider uppercase hover:border-accent-neon focus-visible:border-accent-neon focus-visible:ring-1 focus-visible:ring-ring focus-visible:outline-none"
          >
            Sign out
          </button>
        )
      }
    >
      <p>
        {username} is signed in but has no permission to view anything in this cluster. Ask your
        administrator to add you to a group with access.
      </p>
      {message && <p className="text-foreground">{message}</p>}
    </HostedPanel>
  )
}
