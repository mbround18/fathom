// Imports, usages, adapters and lazy bindings of kit pieces: none of them is a copy.
import { lazy } from 'react'
import { EmptyState, ErrorLine as Alert, H2 } from '@companion/ui/kit'

const Markdown = lazy(() => import('./guide-markdown'))

// An adapter: it wraps the kit's ErrorLine, imported as Alert.
function ErrorLine({ store }) {
  return <Alert message={store.error} />
}

export function Screen({ store }) {
  // Nested declarations are local helpers, not app-wide copies.
  const LoadFailed = () => <p>failed</p>
  function ConfirmDialog() {
    return null
  }
  return (
    <section className={H2}>
      <EmptyState />
      <ErrorLine store={store} />
      <LoadFailed />
      <ConfirmDialog />
      <Markdown />
    </section>
  )
}

export { EmptyState as Empty } from '@companion/ui/kit'
