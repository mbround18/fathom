// @companion/terminal: the terminal, log view and pane states shared by the suite's frontends
// (spec 050). Side-effect free; xterm's stylesheet is `@companion/terminal/terminal.css`.
export { DEFAULT_RING_CAPACITY, RingBuffer } from './ring-buffer'
export {
  defaultDroppedText,
  FOLLOW_THRESHOLD_PX,
  FULL_RENDER_MAX,
  LogView,
  type DroppedCount,
  type LogViewProps,
} from './LogView'
export {
  encodeInput,
  TerminalSurface,
  type TerminalPhase,
  type TerminalStream,
  type TerminalSurfaceProps,
  type TerminalTransport,
} from './TerminalSurface'
export {
  Disconnected,
  Empty,
  ErrorState,
  Loading,
  MetricsUnavailable,
  NoLongerExists,
  NotPermitted,
  StateFrame,
  type StateFrameProps,
} from './PaneStates'
export { YamlDiffView, type YamlDiffViewProps, type YamlEditSession } from './YamlDiffView'
