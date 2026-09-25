import { Empty } from '@/components/shell/PaneStates'

/** Placeholder for the Grafana pane; a later batch fills this in (PaneHost already routes here). */
export default function GrafanaPanel() {
  return <Empty title="Grafana" detail="The Grafana integration is not built in this UI yet." />
}
