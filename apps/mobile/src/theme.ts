export interface Palette {
  bg: string;
  card: string;
  text: string;
  muted: string;
  border: string;
  up: string;
  down: string;
  accent: string;
}

export const light: Palette = {
  bg: '#F5F7FA',
  card: '#FFFFFF',
  text: '#0B1220',
  muted: '#5B6577',
  border: '#DDE2EA',
  up: '#0E9F6E',
  down: '#E02424',
  accent: '#2563EB',
};

export const dark: Palette = {
  bg: '#0B1220',
  card: '#131C2E',
  text: '#E6EAF2',
  muted: '#8A94A8',
  border: '#24304A',
  up: '#22C55E',
  down: '#F05252',
  accent: '#60A5FA',
};

export type ThemePref = 'system' | 'light' | 'dark';

export function resolvePalette(pref: ThemePref, system: 'light' | 'dark' | null | undefined): Palette {
  const mode = pref === 'system' ? (system ?? 'light') : pref;
  return mode === 'dark' ? dark : light;
}
