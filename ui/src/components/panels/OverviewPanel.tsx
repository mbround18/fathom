import { Empty } from '@/components/shell/PaneStates'

/** Placeholder for the Overview pane; a later batch fills this in (PaneHost already routes here). */
export default function OverviewPanel() {
  return <Empty title="Overview" detail="The cluster overview is not built in this UI yet." />
}
