/**
 * The menu bar's items that grey out when they can't act (P1-49): Stop
 * Running Turn with nothing running, Back and Forward at the ends of the
 * history, as the top bar's own Back and Forward already do. While the side
 * panel has a page open, Back and Forward stay on: they may step that page
 * (appMenu.ts `runNativeMenu`), and its history isn't known here. The backend
 * sets them (`app_menu.rs` `set_menu_enabled`). Loaded after launch (App.tsx),
 * so the launch bundle doesn't carry it.
 */
import { createEffect, createRoot } from 'solid-js';
import { api } from './api';
import { chatBusy } from './store/chatRuntime';
import { canNavBack, canNavForward } from './store/runtime';
import { browserTabs } from './store/browser';
import { organizationUndoReady } from './store/organizationUndo';

export interface MenuFacts { busy: boolean; back: boolean; forward: boolean; browser: boolean; undo?: boolean }

export function menuStates(f: MenuFacts): Record<'stop-run' | 'go-back' | 'go-forward' | 'undo-chat-action', boolean> {
  return { 'stop-run': f.busy, 'go-back': f.back || f.browser, 'go-forward': f.forward || f.browser, 'undo-chat-action': !!f.undo };
}

/** Keep the three items in step until the returned function is called. */
export function syncMenuState(): () => void {
  return createRoot(dispose => {
    let last = '';
    createEffect(() => {
      const items = menuStates({ busy: chatBusy(), back: canNavBack(), forward: canNavForward(), browser: browserTabs().length > 0, undo: organizationUndoReady() });
      const key = JSON.stringify(items);
      if (key === last) return;
      last = key;
      api.setMenuEnabled(items).catch(() => { /* not in tauri */ });
    });
    return dispose;
  });
}
