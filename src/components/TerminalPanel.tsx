import { For, Index, Show, createEffect, createSignal, on, onCleanup, onMount } from 'solid-js';
import '../terminalChatLink';
import { Icon } from '../icons';
import { chatAgent, chatCwd, chatPermissionMode, vaultRoot } from '../store/chatRuntime';
import { flashFooter } from '../store';
import { StatePanel } from './ui';
import { keyToBytes, pasteBytes } from '../terminalKeys';
import {
  activeTerminal, closeChatTerminals, closeTerminal, openTerminal, resizeTerminal, setActiveTerminal, terminalRefusal, terminalsOf, typeInto,
  type TerminalTab,
} from '../store/terminals';
import '../styles/terminal-panel.css';

/** The terminal drawer's tab (parity 7.2, D2 and D7): this chat's shells in
 *  the Preview pane. Several terminals, each its own tab inside this one; the
 *  shells and their screens live in `store/terminals.ts`, so leaving this tab
 *  keeps them running. A shell starts only where the chat's permission mode
 *  allows one, in the chat's own folder, with an allowlist environment (the
 *  backend, `terminal.rs`); a mode that stops allowing it ends the chat's
 *  terminals and this tab says why. Every action is a visible button: New
 *  terminal, Copy output, Clear, and a Close on each terminal. */
export function TerminalPanel(props: { chat: string }) {
  let body: HTMLDivElement | undefined;
  const [starting, setStarting] = createSignal(false);
  const tabs = () => terminalsOf(props.chat);
  const refusal = () => terminalRefusal(props.chat);

  const fit = () => measureFit(body);
  async function start() {
    if (starting()) return;
    setStarting(true);
    try {
      const cwd = chatCwd() ?? (await vaultRoot());
      if (!cwd) { flashFooter('× No folder to start a terminal in.'); return; }
      const { rows, cols } = fit();
      await openTerminal(props.chat, { agent: chatAgent(), perm: chatPermissionMode(), cwd }, rows, cols);
    } finally { setStarting(false); }
  }

  // A mode (or agent) that stops allowing a shell ends the chat's terminals
  // and leaves the reason here: `terminalChatLink.ts` watches it, mounted or not.
  // The first open waits for a frame so the pane has its size.
  onMount(() => {
    const frame = requestAnimationFrame(() => { if (!tabs().length && !refusal()) void start(); });
    onCleanup(() => cancelAnimationFrame(frame));
  });

  const copyAll = async () => {
    const t = activeTerminal(props.chat);
    if (!t) return;
    try { await navigator.clipboard.writeText(t.screen.allText()); flashFooter('Terminal output copied'); } catch { flashFooter('× Could not copy the terminal output'); }
  };
  const clear = () => {
    const t = activeTerminal(props.chat);
    if (!t || t.exited()) return;
    t.screen.write('\x1b[3J');
    typeInto(t.id, '\x0c');
  };

  return (
    <div class="term-panel">
      <div class="term-bar">
        <div class="term-tabs" role="tablist" aria-label="Terminals">
          <For each={tabs()}>{t => (
            <span class="term-tab-wrap" classList={{ active: activeTerminal(props.chat)?.id === t.id }}>
              <button type="button" role="tab" class="term-tab" aria-selected={activeTerminal(props.chat)?.id === t.id} title={`${t.label}\n${t.cwd}`} onClick={() => setActiveTerminal(props.chat, t.id)}>
                <span class="term-tab-label">{t.label}</span>
                <Show when={t.exited()}><span class="term-tab-state"> (ended)</span></Show>
              </button>
              <button type="button" class="term-tab-close" aria-label={`Close ${t.label}`} title={`Close ${t.label}: ends its shell and what it started`} onClick={() => closeTerminal(props.chat, t.id)}><Icon name="x" size={12} /></button>
            </span>
          )}</For>
          <button type="button" class="chat-panel-iconbtn" aria-label="New terminal" title="New terminal (up to 4 in this chat)" disabled={starting() || tabs().length >= 4 || !!refusal()} onClick={() => void start()}><Icon name="plus" size={14} /></button>
        </div>
        <div class="term-actions">
          <button type="button" class="chat-panel-iconbtn" aria-label="Copy output" title="Copy this terminal's output" disabled={!activeTerminal(props.chat)} onClick={() => void copyAll()}><Icon name="copy" size={14} /></button>
          <button type="button" class="chat-panel-iconbtn" aria-label="Clear" title="Clear the screen and scrollback" disabled={!activeTerminal(props.chat) || activeTerminal(props.chat)!.exited()} onClick={clear}><Icon name="trash" size={14} /></button>
          <button type="button" class="chat-panel-iconbtn" aria-label="Close all terminals" title="Close every terminal in this chat: ends their shells and what they started" disabled={!tabs().length} onClick={() => closeChatTerminals(props.chat)}><Icon name="x" size={14} /></button>
        </div>
      </div>
      <div class="term-body" ref={body}>
        <Show when={refusal()}>{text => <StatePanel compact kind="empty" title="The terminal is off for this chat">{text()}</StatePanel>}</Show>
        <Show when={!refusal() && !tabs().length}>
          <p class="term-note" role="status">{starting() ? 'Starting a shell…' : 'No terminal is open. Use New terminal to start one in this chat’s folder.'}</p>
        </Show>
        <Show when={!refusal() ? activeTerminal(props.chat) : null} keyed>{tab => <TerminalView tab={tab} />}</Show>
      </div>
    </div>
  );
}

/** Rows and columns that fit `el`, from a probe of the real font. */
function measureFit(el: HTMLElement | undefined): { rows: number; cols: number } {
  if (!el) return { rows: 24, cols: 80 };
  const cell = measureCell(el);
  const style = getComputedStyle(el);
  const w = el.clientWidth - parseFloat(style.paddingLeft || '0') - parseFloat(style.paddingRight || '0');
  const h = el.clientHeight - parseFloat(style.paddingTop || '0') - parseFloat(style.paddingBottom || '0');
  return { cols: Math.max(10, Math.floor(w / cell.w)), rows: Math.max(2, Math.floor(h / cell.h)) };
}

function measureCell(host: HTMLElement): { w: number; h: number } {
  const probe = document.createElement('span');
  probe.className = 'term-measure';
  probe.textContent = 'W'.repeat(20);
  host.appendChild(probe);
  const rect = probe.getBoundingClientRect();
  probe.remove();
  return { w: rect.width / 20 || 7.5, h: rect.height || 17 };
}

function TerminalView(props: { tab: TerminalTab }) {
  const tab = props.tab;
  let viewport: HTMLDivElement | undefined;
  let input: HTMLTextAreaElement | undefined;
  const [cell, setCell] = createSignal({ w: 7.5, h: 17 });
  const [focused, setFocused] = createSignal(false);
  let stick = true;
  // One read per change: the screen bumps `tick` once per frame at most.
  const version = () => tab.tick();
  const rows = () => { version(); return tab.screen.rowsHtml(); };
  const chunks = () => { version(); return tab.screen.scrollbackChunks().slice(); };
  const tail = () => { version(); return tab.screen.scrollbackTail(); };
  // A full-screen program owns the whole view: no scrollback above it.
  const alt = () => { version(); return tab.screen.altScreen; };
  const scrollbackRows = () => (alt() ? 0 : tab.screen.scrollbackCount());
  const cursor = () => { version(); return { row: tab.screen.cursorRow, col: tab.screen.cursorCol, visible: tab.screen.cursorVisible }; };

  const refit = () => {
    if (!viewport) return;
    const c = measureCell(viewport);
    setCell(c);
    const style = getComputedStyle(viewport);
    const w = viewport.clientWidth - parseFloat(style.paddingLeft || '0') - parseFloat(style.paddingRight || '0');
    const h = viewport.clientHeight - parseFloat(style.paddingTop || '0') - parseFloat(style.paddingBottom || '0');
    resizeTerminal(tab.id, Math.max(2, Math.floor(h / c.h)), Math.max(10, Math.floor(w / c.w)));
  };
  onMount(() => {
    refit();
    const observer = new ResizeObserver(() => requestAnimationFrame(refit));
    if (viewport) observer.observe(viewport);
    onCleanup(() => observer.disconnect());
    // Opened by a click: type straight away, unless focus is somewhere the person put it.
    const active = document.activeElement;
    if (!(active instanceof HTMLElement) || !/^(INPUT|TEXTAREA|SELECT)$/.test(active.tagName)) input?.focus({ preventScroll: true });
  });
  // Follow the newest output while the reader is at the bottom.
  createEffect(on(version, () => queueMicrotask(() => { if (stick && viewport) viewport.scrollTop = viewport.scrollHeight; })));
  const onScroll = () => { if (viewport) stick = viewport.scrollHeight - viewport.scrollTop - viewport.clientHeight < cell().h; };
  const toBottom = () => { stick = true; if (viewport) viewport.scrollTop = viewport.scrollHeight; };

  const onKeyDown = (e: KeyboardEvent) => {
    if (e.isComposing) return;
    // Copy a selection of the output; with none, Cmd+C stays the app's.
    if (e.metaKey && !e.ctrlKey && !e.altKey && e.key.toLowerCase() === 'c') {
      const text = window.getSelection()?.toString() ?? '';
      if (text) { e.preventDefault(); void navigator.clipboard.writeText(text).catch(() => {}); }
      return;
    }
    const bytes = keyToBytes(e, { appCursor: tab.screen.appCursorKeys });
    if (bytes === null) return;
    // Escape and Tab belong to the shell here (vim, completion), not to the pane.
    e.preventDefault();
    e.stopPropagation();
    typeInto(tab.id, bytes);
    toBottom();
  };
  const onPaste = (e: ClipboardEvent) => {
    e.preventDefault();
    const text = e.clipboardData?.getData('text/plain') ?? '';
    if (text) { typeInto(tab.id, pasteBytes(text, tab.screen.bracketedPaste)); toBottom(); }
  };
  // Text composed through an input method arrives here, not as key presses.
  const onCompositionEnd = (e: CompositionEvent) => {
    if (e.data) typeInto(tab.id, e.data);
    if (input) input.value = '';
  };
  const onMouseUp = () => { if (!(window.getSelection()?.toString())) input?.focus({ preventScroll: true }); };

  return (
    <div class="term-view" classList={{ focused: focused(), ended: tab.exited() }} role="group" aria-label={`${tab.label}, a shell in ${tab.cwd}`} title="Shift+Tab moves focus out of the terminal">
      <div class="term-viewport" ref={viewport} onScroll={onScroll} onMouseUp={onMouseUp}>
        <div class="term-content" style={{ '--term-cw': `${cell().w}px`, '--term-lh': `${cell().h}px` }} aria-hidden="true">
          <Show when={!alt()}>
            <Index each={chunks()}>{html => <div class="term-chunk" innerHTML={html()} />}</Index>
            <div class="term-chunk" innerHTML={tail()} />
          </Show>
          <Index each={rows()}>{html => <div class="trow" innerHTML={html()} />}</Index>
          <Show when={cursor().visible && !tab.exited()}>
            <span class="term-cursor" style={{ top: `${(scrollbackRows() + cursor().row) * cell().h}px`, left: `${cursor().col * cell().w}px` }} />
          </Show>
        </div>
      </div>
      <textarea ref={input} class="term-input" aria-label={`${tab.label} input`} spellcheck={false} autocapitalize="off" autocomplete="off" autocorrect="off"
        onKeyDown={onKeyDown} onPaste={onPaste} onCompositionEnd={onCompositionEnd} onFocus={() => setFocused(true)} onBlur={() => setFocused(false)} />
    </div>
  );
}
