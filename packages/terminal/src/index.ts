// @companion/terminal: the terminal, log view and pane states shared by the suite's frontends
// (spec 050). Side-effect free; xterm's stylesheet is `@companion/terminal/terminal.css`.
export { DEFAULT_RING_CAPACITY, RingBuffer } from './ring-buffer'
export {
  defaultDroppedText,
  FLASH_MS,
  FOLLOW_THRESHOLD_PX,
  FULL_RENDER_MAX,
  LogView,
  type DroppedCount,
  type LogFocus,
  type LogViewProps,
} from './LogView'
export { LOG_QUERY_HINT, LogExplorer, type LogExplorerProps } from './LogExplorer'
export {
  ANSI_NAMES,
  ansiColorCss,
  ansiCss,
  parseAnsi,
  stripAnsi,
  type AnsiColor,
  type AnsiLine,
  type AnsiRun,
  type AnsiStyle,
} from './ansi'
export {
  fieldsOf,
  LineCache,
  LOG_LEVELS,
  normalizeLevel,
  parseLine,
  prettyJson,
  sharedLineCache,
  textLevel,
  type LogLevel,
  type ParsedLine,
} from './log-parse'
export {
  filterLines,
  highlightRanges,
  matchesQuery,
  parseQuery,
  type FilterResult,
  type LevelCounts,
  type LogQuery,
} from './log-query'
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
