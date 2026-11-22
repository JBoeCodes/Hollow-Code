/** Pure presentation choices, shared by storeless boot, the store and Settings. */
export const TEXT_SCALE_OPTIONS = [
  { value: '90', label: 'Smaller' }, { value: '100', label: 'Default' },
  { value: '112', label: 'Larger' }, { value: '125', label: 'Largest' },
] as const;
export type TextScale = typeof TEXT_SCALE_OPTIONS[number]['value'];
export const TEXT_SCALES = TEXT_SCALE_OPTIONS.map(option => option.value);
export function normalizeTextScale(value: string | null): TextScale {
  return TEXT_SCALES.includes(value as TextScale) ? value as TextScale : '100';
}
