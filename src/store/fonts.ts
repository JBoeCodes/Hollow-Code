// The chosen interface and code fonts (fontChoice.ts), saved per machine and
// applied as `--ui` and `--mono` on the root, the way text size is applied at
// load (store/studio.ts). Default means no override: tokens.css stands.
import { createSignal } from 'solid-js';
import { fontStack, normalizeFontChoice, type FontKind } from '../fontChoice';

const KEY: Record<FontKind, string> = { ui: 'hollow.font.ui', code: 'hollow.font.code' };
const VAR: Record<FontKind, string> = { ui: '--ui', code: '--mono' };

function read(kind: FontKind): string {
  try { return normalizeFontChoice(kind, localStorage.getItem(KEY[kind])); } catch { return 'default'; }
}
export const [uiFont, setUiFontSignal] = createSignal(read('ui'));
export const [codeFont, setCodeFontSignal] = createSignal(read('code'));

function apply(kind: FontKind, choice: string) {
  const stack = fontStack(kind, choice);
  if (typeof document === 'undefined') return;
  if (stack) document.documentElement.style.setProperty(VAR[kind], stack);
  else document.documentElement.style.removeProperty(VAR[kind]);
}
apply('ui', uiFont());
apply('code', codeFont());

export function setFont(kind: FontKind, raw: string) {
  const choice = normalizeFontChoice(kind, raw);
  (kind === 'ui' ? setUiFontSignal : setCodeFontSignal)(choice);
  try { if (choice === 'default') localStorage.removeItem(KEY[kind]); else localStorage.setItem(KEY[kind], choice); } catch { /* memory holds it */ }
  apply(kind, choice);
}
