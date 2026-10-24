/**
 * The `.env` editor (US3, T061). The page only mounts it with text that is safe to show: a file
 * with no secret-looking keys, or one the person revealed (`RevealEnv`, audited). Masked text is
 * never put in here, so it can never be saved over the real values. Import it with `React.lazy`.
 */
import { CodeEditor } from './CodeEditor'
import type { Problem } from './problems'

const NONE: readonly never[] = []

/** `problems`: the server's errors for the `.env` file from the last save attempt. */
export default function EnvEditor({ problems = NONE, ...props }: { value: string; onChange: (text: string) => void; problems?: readonly Problem[] }) {
  return <CodeEditor language="env" label=".env" problems={problems} {...props} />
}
