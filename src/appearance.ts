import type { AppearanceSettings, ThemeProfile } from './types';
export function profile(night = false): ThemeProfile {
  const keys = [
    'window',
    'panel',
    'raised',
    'text',
    'muted',
    'accent',
    'accentText',
    'border',
    'selection',
    'focus',
    'danger',
    'warning',
    'success',
  ];
  const values = night
    ? [
        '#171b1e',
        '#20262a',
        '#2a3237',
        '#e8ebe9',
        '#99a6ac',
        '#75c8ad',
        '#0c211b',
        '#354148',
        '#334b43',
        '#89d7bf',
        '#ec9c91',
        '#d9b86c',
        '#83c5a7',
      ]
    : [
        '#f1f0ec',
        '#ffffff',
        '#e9e8e2',
        '#26332f',
        '#68756f',
        '#276c55',
        '#ffffff',
        '#d6dcd6',
        '#d8e9de',
        '#34795e',
        '#ae4035',
        '#886321',
        '#34745c',
      ];
  return {
    colors: Object.fromEntries(keys.map((key, i) => [key, values[i]])),
    font: 'Segoe UI',
    scale: 1,
    density: 1,
    readerBackground: night ? '#111416' : '#dcded7',
    margin: 24,
    gap: 20,
    filters: { brightness: 1, contrast: 1, warmth: 0, saturation: 1, grayscale: 0, inversion: 0 },
  };
}
export const defaults = (): AppearanceSettings => ({
  active: 'day',
  day: profile(),
  night: profile(true),
});
export function pageFilter(p: ThemeProfile, mode: string): string {
  if (mode !== 'Reader') return 'none';
  const f = p.filters;
  return `brightness(${f.brightness}) contrast(${f.contrast}) sepia(${f.warmth}) saturate(${f.saturation}) grayscale(${f.grayscale}) invert(${f.inversion})`;
}
export function applyTheme(a: AppearanceSettings) {
  const p = a[a.active];
  document.documentElement.dataset.appearance = a.active;
  for (const [k, v] of Object.entries(p.colors))
    document.documentElement.style.setProperty(`--${k}`, v);
  document.documentElement.style.setProperty('--font', p.font);
  document.documentElement.style.setProperty('--scale', String(p.scale));
  document.documentElement.style.setProperty('--density', String(p.density));
  document.documentElement.style.colorScheme = a.active === 'night' ? 'dark' : 'light';
  document.documentElement.style.background = p.colors.window;
}
