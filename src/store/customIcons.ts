import { createSignal } from 'solid-js';
import { api } from '../api';
import { listen } from '../nativeEvents';
import type { CustomIcon, IconTarget } from '../customIcons';

export const [mainAgentIcon, setMainAgentIcon] = createSignal<string | null>(null);
export const [appIcon, setAppIcon] = createSignal<string | null>(null);
/** The icon dialog (overhaul H1, H2): Upload an icon, or Generate a logo. */
export const [iconEditor, setIconEditor] = createSignal<{target: IconTarget; mode: 'upload' | 'generate'} | null>(null);
let started = false;
const serial: Record<IconTarget, number> = { app: 0, mainAgent: 0 };
let stockFavicon: { href: string; type: string } | undefined;
export function useSavedIcon(target: IconTarget, record: CustomIcon | null) {
  serial[target]++;
  const url = record?.pngs[128] ? `data:image/png;base64,${record.pngs[128]}` : null;
  if(target==='mainAgent') setMainAgentIcon(url);
  else {
    setAppIcon(url);
    const link = typeof document === 'undefined' ? null : document.querySelector<HTMLLinkElement>('link[rel="icon"]');
    if(link) {
      stockFavicon ??= { href: link.getAttribute('href') ?? '/hollow-32.png', type: link.type };
      link.href = url ?? stockFavicon.href;
      link.type = url ? 'image/png' : stockFavicon.type;
    }
  }
}
export async function refreshIcon(target: IconTarget) {
  const ticket=++serial[target];
  try { const r=await api.readCustomIcon(target); if(ticket===serial[target]) useSavedIcon(target,r); }
  catch { /* Appearance shows the error and offers explicit recovery. Stock stays available. */ }
}
export function ensureCustomIcons() {
  if(started)return; started=true;
  void refreshIcon('mainAgent');
  void refreshIcon('app');
  // App-lifetime shared state, one listener regardless of the number of indicators.
  void listen<string>('custom-icon-changed', e=> { if(e.payload==='mainAgent'||e.payload==='app')void refreshIcon(e.payload); }).catch(()=>{});
}
