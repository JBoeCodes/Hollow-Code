/** Lazy export palette snapshot, shared by file export and correction pictures. */
import { api } from '../api';
import { lookPairs } from '../canvasLook';
import { colorScheme, themePairs } from '../components/htmlSandbox';

export async function exportPalette(name?: string | null) {
  if (!name) return { scheme: colorScheme(), theme: themePairs() };
  // Re-read at save time: a cached live-frame palette may outlast its file.
  const look = lookPairs(await api.readCustomTheme(name));
  return { scheme: look.scheme, theme: themePairs(look), themeName: name };
}
