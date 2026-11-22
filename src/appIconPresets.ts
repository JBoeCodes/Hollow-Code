import type { CustomIcon } from './customIcons';

export const APP_ICON_PRESETS = [
  { id: 'fold', name: 'Fold', default: true },
  { id: 'threshold', name: 'Threshold' },
  { id: 'cut-glass', name: 'Cut Glass' },
  { id: 'aurora', name: 'Aurora' },
  { id: 'monolith', name: 'Monolith' },
] as const;
export type AppIconPreset = typeof APP_ICON_PRESETS[number]['id'];
export function selectedAppIcon(record: CustomIcon | null): AppIconPreset | 'custom' {
  if (!record) return 'fold';
  return APP_ICON_PRESETS.some(p => p.id === record.preset && p.id !== 'fold')
    ? record.preset as AppIconPreset : 'custom';
}
export const appIconLabel = (record: CustomIcon | null) =>
  APP_ICON_PRESETS.find(p => p.id === selectedAppIcon(record))?.name ?? 'Custom';
