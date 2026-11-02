/**
 * `@companion/ui/kit`: the screen-level pieces the suite's apps share (spec 065,
 * contracts/kit-api.md). The primitives in `../index.ts` never import from here, so an app that
 * does not use the kit does not bundle it.
 */
export { KitProvider, useKit, type GlossaryEntry, type GuideRef, type KitContext } from './provider'
export { matchPath, navigate, setNavigationGuard, useHash, usePath, useSearchParam, type Params } from './router'
export { Link } from './Link'
export { isTransportFailure, type ListHandlers, type ListLoad, type Refusal } from './refusal'
export { RefusalText } from './RefusalText'
export { LoadFailed } from './LoadFailed'
export { LoadState } from './LoadState'
export { ErrorLine } from './ErrorLine'
export { Page } from './Page'
export { PageErrorBoundary } from './PageErrorBoundary'
export { EmptyState } from './EmptyState'
export { emptyVariant, type EmptyVariant } from './empty-variant'
export { NoMatch } from './NoMatch'
export { Dialog } from './Dialog'
export { ConfirmDialog, PreviewDetails, type Preview } from './ConfirmDialog'
export { CopyBlock } from './CopyBlock'
export { copyText } from './clipboard'
export { useReveal } from './reveal'
export {
  Field,
  FIELD,
  FIELD_DENSE,
  FORM_WIDTH,
  FormPanel,
  H1,
  H2,
  H3,
  LABEL,
  LINK,
  SectionIntro,
  Select,
  STATUS,
} from './form'
export { Tabs, type TabItem } from './Tabs'
export { DataTable, type Column } from './DataTable'
export { type RowAction, type RowActions, type SortState, type TableSelection, type TableSort } from './table'
export { DataGrid, type GridColumn, type GridCursor, type RowMenuRequest } from './DataGrid'
export { computeVisibleColumns, type BudgetColumn } from './column-budget'
export { GuideLink } from './GuideLink'
export { PageHeader } from './PageHeader'
export { default as Markdown } from './Markdown'
export { slug, slugger } from './slug'
export { TermTip } from './TermTip'
export { NavShell, type NavGroup, type NavItem } from './NavShell'
