// 013-agent-sdk-ops (US1 MVP): kinds the "Ask AI" tab is offered for. Grounding data collection
// (`src/backend/agent/grounding.rs`) currently only implements Pod grounding — other kinds still
// get a working conversation, just without grounding context yet, so this list is intentionally
// broader than "Pod only" per the feature's "be reasonably inclusive" guidance.
export const AGENT_OPS_KINDS: ReadonlySet<string> = new Set([
  'Pod',
  'Deployment',
  'DaemonSet',
  'StatefulSet',
  'ReplicaSet',
  'Job',
  'CronJob',
  'Service',
])
