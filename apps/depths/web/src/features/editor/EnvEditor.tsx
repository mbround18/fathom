/**
 * The `.env` editor (US3, T061). It shows text that is safe to show: the file with secret-looking
 * values masked, or the real text an admin revealed (`RevealEnv`, audited). Saving masked text is
 * safe: the server puts the real value back on every line still reading `KEY=********` (FR-021).
 * Import it with `React.lazy`.
 */
import { CodeEditor } from './CodeEditor'
import type { Problem } from './problems'

const NONE: readonly never[] = []

/** `problems`: the server's errors for the `.env` file from the last save attempt. */
export default function EnvEditor({ problems = NONE, ...props }: { value: string; onChange: (text: string) => void; problems?: readonly Problem[] }) {
  return <CodeEditor language="env" label=".env" problems={problems} {...props} />
}
