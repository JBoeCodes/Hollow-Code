/**
 * Settings, Keyboard shortcuts: one switch that turns every Hollow shortcut
 * off, and a row per command (shortcuts.ts COMMANDS) to record a new key,
 * clear it or put back its default. Saved by the backend (shortcuts.rs),
 * which also sets the menu bar and the system-wide quick capture / ingest
 * keys. While a row records, the backend pauses every shortcut so the menu
 * bar can't take the keystroke first.
 */
import { showConfirm, toggleShortcutsOverlay } from '../store';
import { For, Show, createSignal, onCleanup, onMount } from 'solid-js';
import { api } from '../api';
import { Switch } from '../components/Switch';
import { Button, PlainErrorText } from '../components/ui';
import {
  COMMANDS, COMMAND_BY_ID, CONTEXT_KEYS, CONTEXT_LABEL, acceptShortcutsView, assignChord, chordFromEvent, chordOf, displayChord,
  setRecordingShortcut, setShortcutWarnings, shortcutSettings, shortcutWarnings, shortcutsEnabled,
  type Chord, type ShortcutCommand, type ShortcutSettings,
} from '../shortcuts';
import { localAltProblem, systemWideProblem } from './shortcutRules';
import { errMsg } from '../utils';
import { localUsesOf } from '../localKeys';
import { reservedReason } from '../macReserved';
import { GO_TARGETS, GO_TARGET_BY_ID, LEADER, assignLeader, leaderKeyOf, leaderKeyOk } from '../keyboardLeader';
import { SHORTCUT_TABS, type ShortcutTab } from './shortcutTabs';
import { setSwipeNav, swipeNav } from '../store/prefs';
import '../styles/shortcuts-panel.css';

const GROUPS: { id: ShortcutCommand['group']; title: string; hint?: string }[] = [
  { id: 'Navigate', title: 'Navigate' },
  { id: 'Create', title: 'Create' },
  { id: 'View', title: 'View' },
  { id: 'Chat', title: 'Chat' },
  { id: 'Anywhere', title: 'Anywhere', hint: 'These work while another app is in front.' },
];

const MODIFIERS = ['Meta', 'Control', 'Shift', 'Alt', 'CapsLock', 'Fn'];

export function ShortcutsPanel() {
  const [recording, setRecording] = createSignal<string | null>(null);
  const [notice, setNotice] = createSignal<{ id: string | null; text: string; error?: boolean } | null>(null);
  const [saving, setSaving] = createSignal(false);
  // Settings PR 5: two tabs, so each fits a screen. Commands first.
  const [tab, setTab] = createSignal<ShortcutTab>('commands');

  onMount(() => { api.getShortcutSettings().then(acceptShortcutsView).catch(() => { /* keep what App loaded */ }); });

  async function save(next: ShortcutSettings): Promise<boolean> {
    setSaving(true);
    try {
      acceptShortcutsView(await api.setShortcutSettings(next));
      return true;
    } catch (e) {
      setNotice({ id: null, text: `Couldn't save: ${errMsg(e)}`, error: true });
      return false;
    } finally {
      setSaving(false);
    }
  }

  /** A tab switch ends a recording, or the window key handler would rebind a command that is no longer shown. */
  function switchTab(next: ShortcutTab) {
    if (recording()) void stopRecording();
    setTab(next);
  }

  async function startRecording(id: string) {
    setNotice(null);
    setRecording(id);
    setRecordingShortcut(true);
    try { await api.suspendShortcuts(true); } catch { /* the webview still ignores them */ }
  }

  async function stopRecording() {
    setRecording(null);
    setRecordingShortcut(false);
    try { setShortcutWarnings(await api.suspendShortcuts(false)); } catch { /* not in tauri */ }
  }

  async function setChord(id: string, chord: Chord) {
    const { next, displaced } = assignChord(shortcutSettings(), id, chord);
    if (!await save(next)) return;
    // An in-surface key on the same chord keeps working inside its surface,
    // so say so instead of letting the two meet by surprise (next-level P18).
    const local = localUsesOf(chord, id).map(c => `${c.label} (${c.surface.toLowerCase()})`);
    const parts = [
      displaced ? `Taken from ${COMMAND_BY_ID[displaced]?.label ?? displaced}, which now has no key.` : '',
      local.length ? `${displayChord(chord)} is also ${local.join(' and ')} there; pick another key to keep both.` : '',
    ].filter(Boolean);
    if (parts.length) setNotice({ id, text: parts.join(' ') });
  }

  onMount(() => {
    // Capture on window: the keystroke never reaches the app's own handler
    // (App.tsx) or a page's.
    const onKey = (e: KeyboardEvent) => {
      const id = recording();
      if (!id) return;
      e.preventDefault();
      e.stopImmediatePropagation();
      if (e.repeat || MODIFIERS.includes(e.key)) return;
      if (e.key === 'Escape' && !e.metaKey && !e.ctrlKey) { void stopRecording(); return; }
      const chord = chordFromEvent(e);
      if (!chord) { setNotice({ id, text: 'Hold ⌘ with a letter, digit, punctuation, arrow or F-key (⌥ and ⇧ are optional).', error: true }); return; }
      const mac = reservedReason(chord, id);
      if (mac) { setNotice({ id, text: `${displayChord(chord)} is macOS's ${mac}. Pick another key.`, error: true }); return; }
      const wide = systemWideProblem(id, chord) ?? localAltProblem(chord);
      if (wide) { setNotice({ id, text: wide, error: true }); return; }
      void (async () => { await setChord(id, chord); await stopRecording(); })();
    };
    window.addEventListener('keydown', onKey, true);
    onCleanup(() => {
      window.removeEventListener('keydown', onKey, true);
      if (recording()) void stopRecording();
    });
  });

  const custom = () => Object.keys(shortcutSettings().keys).length > 0 || Object.keys(shortcutSettings().leader ?? {}).length > 0;
  /** Every custom key goes at once, so it asks first (P0-13). */
  async function resetAll() {
    const n = Object.keys(shortcutSettings().keys).length + Object.keys(shortcutSettings().leader ?? {}).length;
    const ok = await showConfirm({
      title: 'Reset every shortcut to its default?',
      body: `Your ${n === 1 ? 'one custom key goes' : `${n} custom keys go`} back to Hollow's defaults. To get ${n === 1 ? 'it' : 'them'} back you record ${n === 1 ? 'it' : 'each one'} again.`,
      confirmLabel: 'Reset all', danger: true, focusCancel: true,
    });
    if (ok) await save({ ...shortcutSettings(), keys: {}, leader: {} });
  }

  /** A go-to target's key after `g` (five asks A4): one letter or digit,
   *  taken from whichever target had it. */
  async function setLeaderKey(id: string, raw: string, input?: HTMLInputElement) {
    const key = raw.trim().toLowerCase().slice(-1);
    // A refused or failed change puts the key in effect back in the box.
    const undo = () => { if (input) input.value = leaderKeyOf(id, shortcutSettings().leader ?? {}); };
    if (key && !leaderKeyOk(key)) { undo(); setNotice({ id: `go:${id}`, text: 'Use one letter or digit.', error: true }); return; }
    const { next, displaced } = assignLeader(shortcutSettings().leader ?? {}, id, key);
    if (!await save({ ...shortcutSettings(), leader: next })) { undo(); return; }
    setNotice(displaced ? { id: `go:${id}`, text: `Taken from ${GO_TARGET_BY_ID[displaced]?.label ?? displaced}, which now has no key.` } : null);
  }

  return (
    <div class="shortcuts-panel">
      <div class="settings-row">
        <div class="settings-row-label">
          <div class="settings-row-k">Use keyboard shortcuts</div>
          <div class="settings-row-v">
            Off: no Hollow shortcut works. Arrows, Enter and the chat’s send keys still do.
          </div>
        </div>
        <Switch checked={shortcutsEnabled()} label="Use keyboard shortcuts" disabled={saving()}
          onChange={on => void save({ ...shortcutSettings(), enabled: on })} />
      </div>

      <div class="settings-row">
        <div class="settings-row-label">
          <div class="settings-row-k">Swipe to go back and forward</div>
          <div class="settings-row-v">
            Two fingers sideways on the trackpad. The buttons and ⌘[ ⌘] work either way.
          </div>
        </div>
        <Switch checked={swipeNav()} label="Swipe to go back and forward" onChange={on => setSwipeNav(on)} />
      </div>

      <Show when={shortcutWarnings().length}>
        <ul class="shortcuts-warnings" role="status">
          <For each={shortcutWarnings()}>{w => <li>{w}</li>}</For>
        </ul>
      </Show>
      <Show when={notice() && notice()!.id === null}>
        <p class="shortcuts-notice" classList={{ error: !!notice()?.error }} role="status"><PlainErrorText text={notice()!.text} plain={!!notice()!.error} /></p>
      </Show>

      <div class="shortcuts-toolbar">
        <div class="seg-toggle" role="tablist" aria-label="Keyboard shortcuts">
          <For each={SHORTCUT_TABS}>{t => (
            <button class={`seg-toggle-btn ${tab() === t.id ? 'active' : ''}`} role="tab" aria-selected={tab() === t.id} id={`shortcuts-tab-${t.id}`} aria-controls={`shortcuts-panel-${t.id}`}
              onClick={() => switchTab(t.id)}>{t.label}</button>
          )}</For>
        </div>
        <div class="shortcuts-toolbar-actions">
          <Button size="sm" disabled={!custom() || saving() || !!recording()} onClick={() => void resetAll()}>Reset all to defaults</Button>
          {/* Every key in one card, the in-surface ones too (consolidation D):
              it was reached only by ? or the native Help menu. */}
          <Button size="sm" variant="ghost" onClick={() => toggleShortcutsOverlay()}>Show all keys</Button>
        </div>
      </div>

      {/* Both panels stay mounted and the other is hidden, so a Settings search finds a control on either tab; the jump selects its tab. */}
      <div id="shortcuts-panel-commands" role="tabpanel" aria-labelledby="shortcuts-tab-commands" hidden={tab() !== 'commands'}>
      <div class="shortcuts-groups" classList={{ off: !shortcutsEnabled() }}>
        <For each={GROUPS}>
          {group => (
            <section class="shortcuts-group" aria-label={group.title}>
              <div class="settings-subhead">
                <div class="settings-subhead-titles">
                  <h3 class="settings-subhead-title">{group.title}</h3>
                  <Show when={group.hint}><p class="settings-section-hint">{group.hint}</p></Show>
                </div>
              </div>
              <For each={COMMANDS.filter(c => c.group === group.id)}>
                {command => {
                  const chord = () => chordOf(command.id);
                  const changed = () => chord() !== command.default;
                  const isRecording = () => recording() === command.id;
                  return (
                    <div class="settings-row shortcut-row" classList={{ recording: isRecording() }} data-shortcut={command.id}>
                      <div class="settings-row-label">
                        <div class="settings-row-k">{command.label}<Show when={command.when && command.when !== 'anywhere'}><span class="shortcut-context">{CONTEXT_LABEL[command.when!]}</span></Show></div>
                        <Show when={notice()?.id === command.id}>
                          <div class="settings-row-v shortcuts-notice" classList={{ error: !!notice()?.error }} role="status"><PlainErrorText text={notice()!.text} plain={!!notice()!.error} /></div>
                        </Show>
                      </div>
                      <div class="shortcut-controls">
                        <Show when={isRecording()} fallback={
                          <Show when={chord()} fallback={<span class="shortcut-none">None</span>}>
                            <kbd class="shortcut-key">{displayChord(chord())}</kbd>
                          </Show>
                        }>
                          <span class="shortcut-listening" aria-live="polite">Press the new keys… Esc cancels</span>
                        </Show>
                        <Show when={!isRecording()} fallback={<Button size="sm" onClick={() => void stopRecording()}>Cancel</Button>}>
                          <Button size="sm" disabled={saving() || !!recording()} aria-label={`Change the key for ${command.label}`}
                            onClick={() => void startRecording(command.id)}>Change</Button>
                          <Show when={chord()}>
                            <Button size="sm" variant="ghost" disabled={saving() || !!recording()} aria-label={`Remove the key for ${command.label}`}
                              onClick={() => void setChord(command.id, '')}>Remove</Button>
                          </Show>
                          <Show when={changed()}>
                            <Button size="sm" variant="ghost" disabled={saving() || !!recording()} aria-label={`Reset ${command.label} to ${displayChord(command.default) || 'none'}`}
                              onClick={() => void setChord(command.id, command.default)}>Reset</Button>
                          </Show>
                        </Show>
                      </div>
                    </div>
                  );
                }}
              </For>
            </section>
          )}
        </For>
      </div>
      <section class="shortcuts-group" classList={{ off: !shortcutsEnabled() }} aria-label="While the palette is open">
        <div class="settings-subhead">
          <div class="settings-subhead-titles">
            <h3 class="settings-subhead-title">{CONTEXT_LABEL.palette}</h3>
            <p class="settings-section-hint">Read only by the open palette. They cannot be changed.</p>
          </div>
        </div>
        <For each={CONTEXT_KEYS.filter(k => k.when === 'palette')}>{k => (
          <div class="settings-row shortcut-row">
            <div class="settings-row-label"><div class="settings-row-k">{k.label}</div></div>
            <div class="shortcut-controls"><kbd class="shortcut-key">{k.keys}</kbd></div>
          </div>
        )}</For>
      </section>
      </div>

      <div id="shortcuts-panel-go" role="tabpanel" aria-labelledby="shortcuts-tab-go" hidden={tab() !== 'go'}>
      {/* The go-to leader (five asks A4): g, then one key. */}
      <section class="shortcuts-group" classList={{ off: !shortcutsEnabled() }} aria-label="Go to">
        <div class="settings-subhead">
          <div class="settings-subhead-titles">
            <h3 class="settings-subhead-title">Go to: press {LEADER.toUpperCase()}, then a key</h3>
            <p class="settings-section-hint">
              Press {LEADER}, then the key. Esc cancels. Type a letter or digit to change one.
            </p>
          </div>
        </div>
        <For each={GO_TARGETS}>{t => {
          const key = () => leaderKeyOf(t.id, shortcutSettings().leader ?? {});
          return (
            <div class="settings-row shortcut-row" data-shortcut={`go:${t.id}`}>
              <div class="settings-row-label">
                <div class="settings-row-k">{t.label}</div>
                <Show when={notice()?.id === `go:${t.id}`}>
                  <div class="settings-row-v shortcuts-notice" classList={{ error: !!notice()?.error }} role="status"><PlainErrorText text={notice()!.text} plain={!!notice()!.error} /></div>
                </Show>
              </div>
              <div class="shortcut-controls">
                <kbd class="shortcut-key">{LEADER}</kbd>
                <input class="shortcut-leader-input" value={key()} maxLength={2} size={2} disabled={saving() || !shortcutsEnabled()}
                  aria-label={`Key after ${LEADER} for ${t.label}`}
                  onChange={e => void setLeaderKey(t.id, e.currentTarget.value, e.currentTarget)} />
                <Show when={key()}>
                  <Button size="sm" variant="ghost" disabled={saving() || !shortcutsEnabled()} aria-label={`Remove the key after ${LEADER} for ${t.label}`}
                    onClick={() => void setLeaderKey(t.id, '')}>Remove</Button>
                </Show>
                <Show when={key() !== t.key}>
                  <Button size="sm" variant="ghost" disabled={saving()} aria-label={`Reset ${t.label} to ${LEADER} ${t.key}`}
                    onClick={() => void setLeaderKey(t.id, t.key)}>Reset</Button>
                </Show>
              </div>
            </div>
          );
        }}</For>
      </section>
      </div>
    </div>
  );
}
