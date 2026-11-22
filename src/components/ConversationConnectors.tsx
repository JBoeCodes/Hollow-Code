import { createEffect, createMemo, createSignal, onMount, onCleanup, type Accessor } from 'solid-js';
import { Icon } from '../icons';
import { modalOpen } from '../modalGuard';
import { ActionMenu } from './ActionMenu';
import { api } from '../api';
import { chatCwd, chatAgentSessionKey, openSettingsFromOutside } from '../store';
import { connectorChecks, m365Check } from '../store/connectorChecks';
import { turnConnectors, type TurnConnector } from '../connectorCapability';
import type { ConnectorsLibraryView } from '../connectorsLibrary';
import '../styles/conversation-connectors.css';

// Passive library status only. Opening the control rereads, never probes.
export interface TurnConnectorsState {
  library: Accessor<ConnectorsLibraryView | null>;
  error: Accessor<string>;
  /** Re-reads the library (no probe). */
  read: () => Promise<void>;
  rows: Accessor<TurnConnector[]>;
  attaching: Accessor<TurnConnector[]>;
  off: Accessor<TurnConnector[]>;
  /** One line: "2 connectors · 1 available · 1 not confirmed". */
  summary: Accessor<string>;
  /** off (none attach), ok (all available), warn (one needs attention) or neutral. */
  tone: Accessor<'off' | 'ok' | 'warn' | 'neutral'>;
}

export function createTurnConnectors(args: Accessor<{ agent: string; surface?: 'chat' | 'automation'; perm?: string }>): TurnConnectorsState {
  const [library, setLibrary] = createSignal<ConnectorsLibraryView | null>(null);
  const [error, setError] = createSignal('');
  const read = () => api.listConnectors().then(lib => { setLibrary(lib); setError(''); }).catch(e => { setError(e instanceof Error ? e.message : String(e)); });
  onMount(() => { void read(); });
  const rows = createMemo<TurnConnector[]>(() => {
    const a = args();
    return turnConnectors(library()?.connectors ?? [], {
      agent: a.agent, surface: a.surface ?? 'chat', perm: a.perm,
      statuses: connectorChecks(), m365: m365Check(), login: library()?.claudeaiLogin,
    });
  });
  const attaching = () => rows().filter(r => r.attach !== 'off');
  const off = () => rows().filter(r => r.attach === 'off');
  const available = () => attaching().filter(r => r.capability.state === 'available').length;
  const summary = () => {
    const n = attaching().length;
    if (!library()) return error() ? 'Connectors unavailable' : 'Connectors…';
    if (!n) return 'No connectors attach';
    const unverified = n - available();
    return `${n} connector${n === 1 ? '' : 's'}${available() ? ` · ${available()} available` : ''}${unverified ? ` · ${unverified} not confirmed` : ''}`;
  };
  const tone = () => !attaching().length ? 'off' as const : available() === attaching().length ? 'ok' as const : attaching().some(r => r.tone === 'warn' || r.tone === 'error') ? 'warn' as const : 'neutral' as const;
  return { library, error, read, rows, attaching, off, summary, tone };
}

/** Two resource lines; permission choices remain in the access chip. */
export function ComposerResources(props: { state: TurnConnectorsState }) {
  const s = props.state;
  const folder = () => chatCwd() ?? 'The vault';
  const name = () => folder().split('/').filter(Boolean).pop() ?? folder();
  let root!: HTMLSpanElement;
  const [place, setPlace] = createSignal({ width: 320, shift: 0 });
  function prepare() {
    if (root.querySelector('details')?.open) return;
    const width = Math.min(320, window.innerWidth - 16);
    const left = root.getBoundingClientRect().left;
    setPlace({ width, shift: Math.max(8, Math.min(left, window.innerWidth - width - 8)) - left });
    void s.read();
  }
  createEffect(() => { chatAgentSessionKey(); const menu = root?.querySelector('details'); if (menu) menu.open = false; });
  // Keep row identity while a passive library refresh changes its labels.
  const items = [
    { get label() { return `Folder · ${name()}`; }, get title() { return folder(); }, informational: true, action: () => {} },
    { get label() { return `Connectors · ${s.summary()}`; }, get title() { return s.rows().map(r => `${r.name}: ${r.label}`).join('; '); }, action: () => { openSettingsFromOutside('connectors', 'Your connectors'); } },
  ];
  onMount(() => {
    const escape = (e: KeyboardEvent) => {
      const menu = root.querySelector('details');
      if (e.key !== 'Escape' || !menu?.open || e.isComposing || modalOpen() || document.querySelector('.keymap-backdrop')) return;
      e.preventDefault(); e.stopPropagation();
      if (root.contains(document.activeElement)) root.querySelector('summary')?.focus();
      menu.open = false;
    };
    const resize = () => { const menu = root.querySelector('details'); if (menu) menu.open = false; };
    document.addEventListener('keydown', escape, true);
    window.addEventListener('resize', resize);
    onCleanup(() => { document.removeEventListener('keydown', escape, true); window.removeEventListener('resize', resize); });
  });
  return <span ref={root} class="composer-resources" style={{ "--resources-width": `${place().width}px`, "--resources-shift": `${place().shift}px` }} onPointerDown={prepare} onKeyDown={e => { if (['Enter', ' ', 'ArrowDown'].includes(e.key)) prepare(); }}>
    <ActionMenu label={`Folder and connectors: ${s.summary()}`} align="start"
      icon={<Icon name="folder" size={14} />} items={items} />
  </span>;
}
