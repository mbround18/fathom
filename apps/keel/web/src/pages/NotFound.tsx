import { Link } from '@/app/Link'
import { HOME } from '@/app/nav'

export default function NotFound() {
  return (
    <section data-page="not-found" className="flex flex-col items-start gap-2 p-4">
      <h1 className="font-display text-sm font-semibold tracking-wider uppercase">Not found</h1>
      <Link to={HOME} className="text-xs text-primary underline-offset-4 hover:underline">
        Back to runs
      </Link>
    </section>
  )
}
