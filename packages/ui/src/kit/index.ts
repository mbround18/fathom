/**
 * `@companion/ui/kit`: the screen-level pieces the suite's apps share (spec 065,
 * contracts/kit-api.md). The primitives in `../index.ts` never import from here, so an app that
 * does not use the kit does not bundle it.
 */
export { KitProvider, useKit, type GlossaryEntry, type GuideRef, type KitContext } from './provider'
export { matchPath, navigate, setNavigationGuard, useHash, usePath, useSearchParam, type Params } from './router'
export { Link } from './Link'
