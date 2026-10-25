/**
 * The compose file editor (US3, T061): YAML highlighting, and the problems the page found
 * (client YAML errors, server compose errors) marked at their lines. Import it with `React.lazy`.
 */
import { CodeEditor } from './CodeEditor'
import type { Problem } from './problems'

export default function ComposeEditor(props: {
  value: string
  onChange: (text: string) => void
  label: string
  problems: readonly Problem[]
}) {
  return <CodeEditor language="yaml" {...props} />
}
