// M2 PR 4: a remote chat row's right-click menu (the machine strip's
// popover, Quick chats): Open, Stop while it runs, Copy conversation ID; what
// only its home Mac can do is listed, disabled. Its own small module, so
// Quick chats doesn't load the strip for it.
import type { MachineRef, PeerSessionRow } from '../generated/ipc';
import { invoke } from '../api';
import { flashFooter, openContextMenu } from '../store';
import type { ContextMenuItem } from '../store/overlays';
import { remoteMenu } from './machineStrip';

export function remoteRowMenuItems(home: MachineRef, r: PeerSessionRow, stale: boolean, open: () => void): ContextMenuItem[] {
  return remoteMenu(r, home, stale).map(entry => ({
    label: entry.label,
    title: entry.title,
    disabled: !entry.enabled,
    action: entry.kind === 'open' ? open
      : entry.kind === 'copy' ? async () => {
        try { await navigator.clipboard.writeText(r.key); flashFooter('Conversation ID copied'); } catch { flashFooter('× Could not copy', 2500); }
      }
      : entry.kind === 'stop' ? async () => {
        try { await invoke<boolean>('peer_stop', { machineId: home.machineId, key: r.key }); flashFooter(`Stopped on ${home.name}`); } catch (err) { flashFooter(`× ${(err as { message?: string })?.message ?? 'Could not stop'}`, 3500); }
      }
      : undefined,
  }));
}

export function openRemoteRowMenu(e: MouseEvent, home: MachineRef, r: PeerSessionRow, stale: boolean, open: () => void): void {
  openContextMenu(e, remoteRowMenuItems(home, r, stale, open));
}
