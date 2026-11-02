/**
 * Catches a page that throws while rendering, so the shell and nav stay up: "Something went wrong
 * showing this page." with the error's message and a Reload. The route table keys it on the path,
 * so moving to another page clears it. `data-pane-state="error"`.
 *
 * `resetKey` (spec 065) clears it without a re-mount: a pane that keeps its boundary passes what it
 * shows (its target), and a new value renders the children again.
 */
import { ArrowClockwiseIcon, WarningOctagonIcon } from '@phosphor-icons/react'
import { Button } from '../button'
import { Component, type ReactNode } from 'react'

interface State {
  error: Error | null
}

interface Props {
  children: ReactNode
  /** A new value clears the error. */
  resetKey?: unknown
}

export class PageErrorBoundary extends Component<Props, State> {
  override state: State = { error: null }

  static getDerivedStateFromError(error: unknown): State {
    return { error: error instanceof Error ? error : new Error(String(error)) }
  }

  override componentDidUpdate(prev: Props) {
    if (this.state.error && !Object.is(prev.resetKey, this.props.resetKey)) this.setState({ error: null })
  }

  override render() {
    const { error } = this.state
    if (!error) return this.props.children
    return (
      <div data-pane-state="error" className="flex min-w-0 flex-col items-start gap-2 p-4">
        <p
          role="alert"
          className="flex min-w-0 items-start gap-1.5 border border-status-err/40 px-2 py-1.5 text-sm text-status-err"
        >
          <WarningOctagonIcon aria-hidden className="mt-0.5 size-4 shrink-0" />
          <span className="min-w-0 break-words">
            <span className="font-medium">Something went wrong showing this page. </span>
            <span data-error-message className="whitespace-pre-wrap">
              {error.message}
            </span>
          </span>
        </p>
        <Button
          type="button"
          variant="outline"
          size="xs"
          onClick={() => {
            location.reload()
          }}
        >
          <ArrowClockwiseIcon aria-hidden />
          Reload
        </Button>
      </div>
    )
  }
}
