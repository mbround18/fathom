import { Empty } from '@/components/shell/PaneStates'

/** Placeholder for the Argo pane; a later batch fills this in (PaneHost already routes here). */
export default function ArgoPanel() {
  return <Empty title="Argo" detail="The Argo integration is not built in this UI yet." />
}
