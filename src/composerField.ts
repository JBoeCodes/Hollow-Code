/** A rich editor exposes source offsets, just like the plain message box. */
export interface RichComposerField {
  element: HTMLElement;
  value: string;
  selectionStart: number;
  selectionEnd: number;
  readonly isConnected: boolean;
  readonly style: CSSStyleDeclaration;
  readonly scrollHeight: number;
  readonly clientWidth: number;
  focus(options?: FocusOptions): void;
  setSelectionRange(from: number, to: number): void;
  getClientRects(): DOMRectList;
  closest(selector: string): Element | null;
  dispatchEvent(event: Event): boolean;
  replaceText(from: number, to: number, text: string): void;
  caretEdges(): { atTop: boolean; atBottom: boolean };
}
export type ComposerField = HTMLTextAreaElement | RichComposerField;
export type DictationField = ComposerField | HTMLInputElement;
export const isRichField = (field: DictationField): field is RichComposerField => 'element' in field;
export const fieldElement = (field: DictationField): HTMLElement => isRichField(field) ? field.element : field;

const fields = new Map<HTMLElement, RichComposerField>();
export function registerRichField(field: RichComposerField): () => void {
  fields.set(field.element, field);
  return () => { if (fields.get(field.element) === field) fields.delete(field.element); };
}
export function richFieldAt(element: Element | null): RichComposerField | null {
  for (const [host, field] of fields) if (host === element || !!element && host.contains(element)) return field;
  return null;
}
export function visibleRichField(): RichComposerField | null {
  for (const [host, field] of fields) {
    if (host.isConnected && !host.closest('[hidden],[inert]') && host.getClientRects().length) return field;
  }
  return null;
}
