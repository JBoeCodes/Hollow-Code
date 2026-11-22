import { Show, createSignal, onMount, untrack } from 'solid-js';
import { api } from '../api';
import { flashFooter } from '../store';
import type { ChatLimits } from '../types';
import { Mini } from './MergedRow';
import { CHAT_LIMIT_MAX, chatLimitDrafts, parseChatLimit, serialSaver, type CarriedEdits } from '../chatLimits';

/** Settings, Chat, Chats at once (2026-10-20): how many agent chat
 *  turns the run pump starts at once across Hollow and in one workspace's
 *  own folder (Rust `chat_limits`). Every agent turn counts toward the
 *  first, automations and swarm parts included. A value that isn't a whole
 *  number in range is refused beside its input and the saved one comes
 *  back; saves go one at a time on the last saved state. Only the person
 *  changes these (the setter is never an agent action). */
export function ChatLimitsRows() {
  const [s, setS] = createSignal<ChatLimits | null>(null);
  const [errors, setErrors] = createSignal<Partial<Record<keyof ChatLimits, string>>>({});
  // The inputs stay mounted and keep their own text. Every acknowledged
  // answer sets a field unless it has a newer edit still unsettled
  // (`chatLimitDrafts`, second delta review): an untouched field follows a
  // change made elsewhere, and a refusal puts a field back only when the
  // refused edit is still its latest.
  const inputs: Partial<Record<keyof ChatLimits, HTMLInputElement>> = {};
  const drafts = chatLimitDrafts();
  // Reads and writes go in one line (`serialSaver`): a re-read after a
  // refused save can't land after, and undo, a later save.
  const saver = serialSaver<ChatLimits, CarriedEdits>(
    next => api.setChatLimits(next),
    () => api.getChatLimits(),
    (saved, carried) => {
      setS(saved);
      const set = drafts.settle(carried ?? {}, saved);
      for (const key of Object.keys(set) as Array<keyof ChatLimits>) {
        const el = inputs[key];
        if (el && el.value !== String(set[key])) el.value = String(set[key]);
      }
    },
    e => flashFooter(`× ${e instanceof Error ? e.message : String(e)}`, 4000),
  );
  onMount(() => { void saver.load(); });
  const input = (key: keyof ChatLimits, label: string) => {
    const id = `chat-limit-${key}`;
    return (
      <div class="settings-mini-field">
        <input id={id} class="settings-input" type="number" min={1} max={CHAT_LIMIT_MAX[key]} step={1}
          ref={el => { inputs[key] = el; }}
          value={untrack(() => s()![key])} aria-label={label}
          aria-invalid={errors()[key] ? 'true' : undefined}
          aria-describedby={errors()[key] ? `${id}-error` : undefined}
          style={{ width: '4.5rem' }}
          onInput={() => drafts.typing(key)}
          onChange={(e) => {
            const parsed = parseChatLimit(e.currentTarget.value, CHAT_LIMIT_MAX[key]);
            if ('error' in parsed) {
              setErrors({ ...errors(), [key]: parsed.error });
              drafts.abandon(key);
              e.currentTarget.value = String(drafts.shown(key, s()![key]));
              return;
            }
            setErrors({ ...errors(), [key]: undefined });
            void saver.save({ [key]: parsed.value }, drafts.edit(key, parsed.value));
          }} />
        <Show when={errors()[key]}>
          {(msg) => <span id={`${id}-error`} class="settings-row-v" role="alert">{msg()}</span>}
        </Show>
      </div>
    );
  };
  // The three limits share one row (Settings PR 5; Subagents per chat joined
  // 2026-11-12); each input keeps its name and its own refusal text.
  return (
    <Show when={s()}>
      <div class="settings-row">
        <div class="settings-row-label">
          <div class="settings-row-k">Chats at once</div>
          <div class="settings-row-v">
            Memory admits up to 60. Overall is the fallback; workspace and subagent limits apply.
          </div>
        </div>
        <div class="settings-actions">
          <Mini label="Overall" control="Chats running at once">{input('chatsAtOnce', 'Chats running at once')}</Mini>
          <Mini label="Per workspace" control="Chats at once in one workspace">{input('chatsPerWorkspace', 'Chats at once in one workspace')}</Mini>
          <Mini label="Subagents per chat" control="Subagents running at once from one chat">{input('subagentsPerChat', 'Subagents running at once from one chat')}</Mini>
        </div>
      </div>
    </Show>
  );
}
