// "How background work runs" (plan completion L2, assignment item 3): the
// four things that run without being asked, side by side with the same five
// questions, and a visible route to each one's settings. Content and sources:
// `backgroundWork.ts`. Tonight's queue was a fifth row; that screen is gone.
// A disclosure on Automations, also opened from the Main agent's side band
// (`openBackgroundWork`).
import { For } from 'solid-js';
import { BACKGROUND_COMMON, BACKGROUND_SYSTEMS, type BackgroundSystem } from '../backgroundWork';
import { backgroundWorkOpen, openAutomationsView, openAutomationWindows, setBackgroundWorkOpen } from '../store/automationsViews';
import { openSettingsFromOutside } from '../store';
import { Button } from './ui';
import '../styles/background-work.css';

function goTo({ target, control }: BackgroundSystem['change']) {
  if (target === 'settingsAutomation') { openSettingsFromOutside('automation', control); return; }
  if (control === 'Automation windows') { void openAutomationWindows('windows'); return; }
  openAutomationsView('automations');
}

export function BackgroundWorkExplainer() {
  return (
    <details class="background-work" open={backgroundWorkOpen()} onToggle={e => setBackgroundWorkOpen(e.currentTarget.open)}>
      <summary>How background work runs</summary>
      {/* Its own scroll area: the page above the list doesn't scroll, so at
          the native minimum window an open explainer must not push past it. */}
      <div class="background-work-body" tabIndex={0} role="region" aria-label="How background work runs">
      <p class="background-work-lead">Four things in Hollow run without you pressing anything. They differ in when they run, what they see, what they may do on their own and what they leave behind.</p>
      <div class="background-work-grid">
        <For each={BACKGROUND_SYSTEMS}>
          {sys => (
            <section class="background-work-system" data-system={sys.id} aria-labelledby={`background-work-${sys.id}`}>
              <h4 id={`background-work-${sys.id}`}>{sys.name}</h4>
              <p class="background-work-purpose">{sys.purpose}</p>
              <dl>
                <div><dt>When</dt><dd>{sys.when}</dd></div>
                <div><dt>What it sees</dt><dd>{sys.scope}</dd></div>
                <div><dt>What it may do</dt><dd>{sys.authority}</dd></div>
                <div><dt>What it leaves</dt><dd>{sys.outputs}</dd></div>
              </dl>
              <Button size="sm" variant="ghost" onClick={() => goTo(sys.change)}>{sys.change.label}</Button>
            </section>
          )}
        </For>
      </div>
      <p class="background-work-common">{BACKGROUND_COMMON}</p>
      </div>
    </details>
  );
}
