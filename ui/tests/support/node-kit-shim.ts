/**
 * Spec files import Fathom's descriptor registry (`src/resources`) for nav and fixture data, and
 * since spec 065 that graph reaches `@companion/ui/kit`, whose router registers a `popstate`
 * listener on `globalThis` at import. Playwright runs specs in Node, which has no
 * `addEventListener` on `globalThis`, so every config loads this first: Node never navigates,
 * and a no-op listener is all the kit needs to import there.
 */
const g = globalThis as { addEventListener?: unknown }
if (typeof g.addEventListener !== 'function') g.addEventListener = () => {}
export {}
