import { invoke, type BrowserBounds } from './api';
import { kitDuration } from './motionKit';

export interface PreviewInspection { tabId: string; gen: number; seq: number; visible: boolean }

/** A notification only invalidates the native marker's presentation. Actual
 * coordinates and the matching share stay in the native browser owner. */
export function inspectionForTab(event: PreviewInspection, tab: { id: string; gen: number; loading: boolean } | undefined): boolean {
  return !!tab && !tab.loading && event.tabId === tab.id && event.gen === tab.gen;
}

export function showPreviewBrowser(tabId: string | null, bounds: BrowserBounds | null, element: HTMLElement): Promise<void> {
  const style = getComputedStyle(element);
  return invoke('browser_show', { tabId, bounds, cursor: tabId ? {
    durationMs: kitDuration(element),
    color: style.getPropertyValue('--primary').trim(),
    background: style.getPropertyValue('--surface-card').trim(),
    ink: style.getPropertyValue('--ink').trim(),
  } : null });
}
