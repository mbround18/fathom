export interface SuitePalette {
  background: string
  card: string
  foreground: string
  mutedForeground: string
  accentNeon: string
  accent2: string
  statusOk: string
  statusWarn: string
  statusErr: string
  border: string
}

export const NEON_NOIR_DARK: Readonly<SuitePalette> = {
  background: '#070b1a',
  card: '#0c1228',
  foreground: '#d6f6ff',
  mutedForeground: '#8a9cc9',
  accentNeon: '#3ef2ff',
  accent2: '#ff2bd6',
  statusOk: '#39ffb0',
  statusWarn: '#ffd23f',
  statusErr: '#ff4d6d',
  border: 'rgba(196, 198, 255, 0.28)',
}

export const BLUEPRINT_LIGHT: Readonly<SuitePalette> = {
  background: '#eef1fb',
  card: '#ffffff',
  foreground: '#0b1030',
  mutedForeground: '#4a5580',
  accentNeon: '#006a80',
  accent2: '#a0107f',
  statusOk: '#007a55',
  statusWarn: '#8a5a00',
  statusErr: '#b00030',
  border: '#0b1030',
}

export const SUITE_PALETTES = {
  dark: NEON_NOIR_DARK,
  light: BLUEPRINT_LIGHT,
} as const
