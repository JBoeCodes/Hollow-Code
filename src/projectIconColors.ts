/**
 * Project monograms and colors (t3code parity 4.4, A8). A project with no app
 * artwork and no GitHub owner shows its first letters on a quiet tint instead
 * of a folder. The tint is a hue from this fixed set, chosen from the project's
 * id so it never changes, or the one the person picked from the project's menu
 * (kept per Mac in `store/projectColors.ts`). Hollow's own hues and lightness
 * (the same hsl scheme as the connector icons), nothing borrowed.
 */
export interface ProjectColor { id: string; name: string; hue: number }

export const PROJECT_COLORS: readonly ProjectColor[] = [
  { id: 'rust', name: 'Rust', hue: 14 },
  { id: 'amber', name: 'Amber', hue: 38 },
  { id: 'olive', name: 'Olive', hue: 75 },
  { id: 'green', name: 'Green', hue: 140 },
  { id: 'teal', name: 'Teal', hue: 180 },
  { id: 'blue', name: 'Blue', hue: 215 },
  { id: 'indigo', name: 'Indigo', hue: 255 },
  { id: 'violet', name: 'Violet', hue: 285 },
  { id: 'rose', name: 'Rose', hue: 335 },
];

/** The color a project id gets when nobody picked one (a stable hash). */
export function defaultProjectColor(id: string): ProjectColor {
  let h = 0;
  for (let i = 0; i < id.length; i++) h = (h * 31 + id.charCodeAt(i)) >>> 0;
  return PROJECT_COLORS[h % PROJECT_COLORS.length];
}

/** The picked color (by id), else the default one. An unknown id falls back too. */
export function projectColor(id: string, picked: string | undefined): ProjectColor {
  return PROJECT_COLORS.find(c => c.id === picked) ?? defaultProjectColor(id);
}

/** The next color in the set after `current`, for the menu's Change color. */
export function nextProjectColor(current: ProjectColor): ProjectColor {
  return PROJECT_COLORS[(PROJECT_COLORS.findIndex(c => c.id === current.id) + 1) % PROJECT_COLORS.length];
}

/** One or two letters: the initials of the first two words, else the first two
 *  letters of a single word. Letters and digits only; "?" when there are none. */
export function monogram(title: string): string {
  const words = title.split(/[^\p{L}\p{N}]+/u).filter(Boolean);
  if (!words.length) return '?';
  const letters = words.length > 1 ? [...words[0]][0] + [...words[1]][0] : [...words[0]].slice(0, 2).join('');
  return letters.toUpperCase();
}
