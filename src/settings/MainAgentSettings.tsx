import { Show, createResource, createSignal, onMount } from 'solid-js';
import { api } from '../api';
import { flashFooter, requestOpenSettingsSection } from '../store';
import { Switch } from '../components/Switch';
import { NewerModelHint } from '../components/NewerModelHint';
import { loadCatalog } from '../modelCatalog';
import { skippedWakeWords, todaySpendLine, type MainAgentSettings } from '../mainAgent';
import { mainAgentStatus, refreshMainAgentStatus } from '../store/mainAgent';
import { checkInWords, spendGauge } from '../mainAgentSpace';
import { Mini } from './MergedRow';
import { MoreFold } from './settingsLayout';

/** Settings, Background work, Main agent (main agent M3–M5): its check-ins,
 *  hours, daily hard stop, whether unattended turns act on their own, and
 *  how big a swarm may be. Only the person changes these (the setter is
 *  never an agent action). */
export function MainAgentSettingsRows() {
  // Flags a saved swarm model whose Claude family has a newer one.
  const [claudeCatalog] = createResource(() => loadCatalog('claude'));
  const [s, setS] = createSignal<MainAgentSettings | null>(null);
  onMount(() => { api.getMainAgentSettings().then(setS).catch(() => setS(null)); void refreshMainAgentStatus(); });
  // Its day, moved here from the COO page's header band (removed 2026-11-11).
  const day = () => {
    const st = mainAgentStatus();
    if (!st) return null;
    const g = spendGauge(st);
    return {
      checks: checkInWords(st),
      spend: todaySpendLine(st, g.tone === 'stop' ? g.words.split(': ')[1] ?? 'stopped' : undefined),
      acts: `${st.actOnOwn ? 'Acts on its own' : 'Suggests only'} · ${st.suggestions.taken} of ${st.suggestions.answered} answered suggestions taken`,
      skipped: st.wakeSkipped ? skippedWakeWords(st.wakeSkipped) : null,
    };
  };
  const save = async (patch: Partial<MainAgentSettings>) => {
    const cur = s();
    if (!cur) return;
    try { setS(await api.setMainAgentSettings({ ...cur, ...patch })); }
    catch (e) {
      flashFooter(`× ${e instanceof Error ? e.message : String(e)}`, 4000);
      // Put the inputs back to what is saved.
      setS(null); setS(await api.getMainAgentSettings().catch(() => cur));
    }
  };
  const num = (val: number, on: (n: number) => void, min: number, max: number, label: string, step = 1) => (
    <input class="settings-input" type="number" min={min} max={max} step={step} value={val} aria-label={label}
      style={{ width: '4.5rem' }}
      onChange={(e) => { const n = Number(e.currentTarget.value); if (Number.isFinite(n)) on(n); }} />
  );
  return (
    <Show when={s()}>
      {(cur) => (
        <>
          <Show when={day()}>{d => (
            <div class="settings-row">
              <div class="settings-row-label">
                <div class="settings-row-k">Today</div>
                <div class="settings-row-v">{d().checks}</div>
                <div class="settings-row-v">{d().spend}</div>
                <div class="settings-row-v">{d().acts}</div>
                <Show when={d().skipped}>{w => <div class="settings-row-v" role="status">{w()}</div>}</Show>
              </div>
            </div>
          )}</Show>
          <div class="settings-row">
            <div class="settings-row-label">
              <div class="settings-row-k">COO check-ins</div>
              <div class="settings-row-v">
                Reviews what waits on you at each check-in. With nothing new it spends nothing.
              </div>
            </div>
            <Switch checked={cur().checkIns} label="COO check-ins" onChange={() => void save({ checkIns: !cur().checkIns })} />
          </div>
          <Show when={cur().checkIns}>
            <div class="settings-row">
              <div class="settings-row-label"><div class="settings-row-k">Check in every (minutes)</div></div>
              <div class="settings-actions">{num(cur().intervalMin, n => void save({ intervalMin: n }), 15, 240, 'Minutes between check-ins')}</div>
            </div>
            {/* Hours and working days share a row (Settings PR 5). */}
            <div class="settings-row">
              <div class="settings-row-label">
                <div class="settings-row-k">Check-in hours</div>
                <div class="settings-row-v">Local time, from the first hour up to the second, on working days or every day.</div>
              </div>
              <div class="settings-actions">
                {num(cur().startHour, n => void save({ startHour: n }), 0, 23, 'First hour')}
                <span class="settings-row-v">to</span>
                {num(cur().endHour, n => void save({ endHour: n }), 1, 24, 'Last hour')}
                <Mini label="Working days only">
                  <Switch checked={cur().weekdaysOnly} label="Check in on working days only" onChange={() => void save({ weekdaysOnly: !cur().weekdaysOnly })} />
                </Mini>
              </div>
            </div>
          </Show>
          <div class="settings-row">
            <div class="settings-row-label">
              <div class="settings-row-k">Daily hard stop</div>
              <div class="settings-row-v">The COO and its subagents stop for the day at the cap. Edited only in Usage.</div>
            </div>
            <div class="settings-actions">
              <button type="button" class="btn-link" onClick={() => requestOpenSettingsSection('usage', 'COO daily hard stop')}>Set in Usage → Budgets</button>
            </div>
          </div>
          <div class="settings-row">
            <div class="settings-row-label">
              <div class="settings-row-k">Act on its own</div>
              <div class="settings-row-v">
                On: it may start chats, update tasks and write in its own folder. Off: it only suggests.
              </div>
              {/* Risky: it acts without asking, so its limits stay one click away. */}
              <MoreFold><div class="settings-row-v">A check-in or report may start and follow up chats (at most accept-edits). It never builds, installs, changes settings, deletes or does anything that leaves this Mac.</div></MoreFold>
            </div>
            <Switch checked={cur().actOnOwn} label="Let the COO act on its own" onChange={() => void save({ actOnOwn: !cur().actOnOwn })} />
          </div>
          <div class="settings-row">
            <div class="settings-row-label">
              <div class="settings-row-k">Start fresh after an hour away</div>
              <div class="settings-row-v">
                A COO chat of 350K or more starts a fresh session after an hour away. Claude only.
              </div>
            </div>
            <Switch checked={cur().freshAfterAway !== false} label="Start fresh after an hour away" onChange={() => void save({ freshAfterAway: cur().freshAfterAway === false })} />
          </div>
          <div class="settings-row">
            <div class="settings-row-label">
              <div class="settings-row-k">Largest swarm (chats)</div>
              <div class="settings-row-v">Only when you ask for one.</div>
            </div>
            <div class="settings-actions">{num(cur().swarmMax, n => void save({ swarmMax: n }), 1, 100, 'Largest swarm')}</div>
          </div>
          <div class="settings-row">
            <div class="settings-row-label">
              <div class="settings-row-k">Swarm parts at once</div>
              <div class="settings-row-v">Parts count toward Chats at once. Parts in build workspaces stay at two.</div>
            </div>
            <div class="settings-actions">{num(cur().swarmConcurrency, n => void save({ swarmConcurrency: n }), 1, 10, 'Swarm parts at once')}</div>
          </div>
          <div class="settings-row">
            <div class="settings-row-label">
              <div class="settings-row-k">Swarm model</div>
              <div class="settings-row-v">
                What swarm parts run on unless the COO picks another. Empty: the newest Claude Haiku.
              </div>
              <NewerModelHint value={cur().swarmModel} catalog={claudeCatalog()} onUse={id => void save({ swarmModel: id })} />
            </div>
            <div class="settings-actions">
              <input class="settings-input" type="text" value={cur().swarmModel} aria-label="Swarm model" placeholder="agent default"
                style={{ width: '11rem' }}
                onChange={(e) => void save({ swarmModel: e.currentTarget.value.trim() })} />
            </div>
          </div>
        </>
      )}
    </Show>
  );
}
