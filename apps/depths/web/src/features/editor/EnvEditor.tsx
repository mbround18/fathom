/**
 * The `.env` editor (US3, T061). The page only mounts it with text that is safe to show: a file
 * with no secret-looking keys, or one the person revealed (`RevealEnv`, audited). Masked text is
 * never put in here, so it can never be saved over the real values. Import it with `React.lazy`.
 */
import { CodeEditor } from './CodeEditor'

const NONE: readonly never[] = []

export default function EnvEditor(props: { value: string; onChange: (text: string) => void }) {
  return <CodeEditor language="env" label=".env" problems={NONE} {...props} />
}
