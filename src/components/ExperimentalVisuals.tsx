import { createEffect, createSignal, onCleanup, onMount } from 'solid-js';
import { experimentalVisuals, setExperimentalActive, studioMotion, studioTexture } from '../store/studio';
import { windowAway } from '../store/noticeTimers';
import { chatBusy, backgroundChatKeys } from '../store/runtime';
import { runQueue } from '../store/runQueue';
import { mainAgentNow } from '../store/mainAgent';
import { motionQuery, motionReduced } from '../motion';
import { experimentalAllowed, experimentalState, textureAllowed } from '../experimentalVisuals';
import '../styles/experimental-visuals.css';

/** One lazy policy owner. Static identity has no frame loop or pointer work. */
export default function ExperimentalVisuals() {
  const [epoch, refresh] = createSignal(0);
  const root = document.documentElement;
  const contrast = matchMedia('(prefers-contrast: more), (forced-colors: active)');
  const transparency = matchMedia('(prefers-reduced-transparency: reduce)');
  createEffect(() => {
    epoch(); studioMotion();
    const policy = {enabled:experimentalVisuals(), texture:studioTexture(), reduced:motionReduced(), away:windowAway(), hidden:document.hidden, focused:document.hasFocus(), contrast:contrast.matches, transparency:transparency.matches};
    const enabled = experimentalAllowed(policy);
    setExperimentalActive(enabled);
    root.dataset.experimentalActive = String(enabled);
    root.dataset.textureActive = String(textureAllowed(policy));
  });
  createEffect(() => {
    const main = mainAgentNow();
    root.dataset.experimentalState = experimentalState(chatBusy() || backgroundChatKeys().length > 0 || runQueue().some(r=>r.status==='running') || main.state==='working' || main.state==='night', main.state==='waiting');
  });
  onMount(() => {
    const update = () => { refresh(n=>n+1); queueMicrotask(()=>refresh(n=>n+1)); };
    const queries = [motionQuery(),contrast,transparency];
    for (const q of queries) q?.addEventListener('change',update);
    window.addEventListener('focus',update); window.addEventListener('blur',update);
    document.addEventListener('visibilitychange',update);
    update();
    onCleanup(() => {
      setExperimentalActive(false);
      delete root.dataset.experimentalActive; delete root.dataset.experimentalState; delete root.dataset.textureActive;
      for (const q of queries) q?.removeEventListener('change',update);
      window.removeEventListener('focus',update); window.removeEventListener('blur',update);
      document.removeEventListener('visibilitychange',update);
    });
  });
  return null;
}
