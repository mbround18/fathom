/** Neon-noir palette for the cytoscape graph (hex: cytoscape cannot parse the oklch tokens). */
export interface GraphPalette {
  fg: string
  edge: string
  accent: string
  healthy: string
  degraded: string
  unhealthy: string
  unknown: string
  kinds: Record<string, string>
  fallback: string
}

const dark: GraphPalette = {
  fg: '#e6ecff',
  edge: '#5a6aa0',
  accent: '#3ef2ff',
  healthy: '#39ffb0',
  degraded: '#ffd23f',
  unhealthy: '#ff4d6d',
  unknown: '#8a9cc9',
  fallback: '#182046',
  kinds: {
    Node: '#2b3a7a',
    Namespace: '#3a2a6e',
    Deployment: '#1f4d6e',
    StatefulSet: '#1f5a6e',
    DaemonSet: '#4a2a6e',
    ReplicaSet: '#2a3f5f',
    HorizontalPodAutoscaler: '#6e2a5a',
    Service: '#2a5f4f',
    Pod: '#182046',
  },
}

const light: GraphPalette = {
  ...dark,
  fg: '#0b1030',
  edge: '#7a86b0',
  accent: '#006a80',
  healthy: '#007a55',
  degraded: '#8a5a00',
  unhealthy: '#b00030',
  unknown: '#4a5580',
  fallback: '#d8def5',
  kinds: Object.fromEntries(Object.keys(dark.kinds).map((k) => [k, '#d8def5'])),
}

export const paletteFor = (isDark: boolean): GraphPalette => (isDark ? dark : light)
