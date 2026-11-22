import doorway from '../assets/brand/hollow-mark-96.png';
import { onMount } from 'solid-js';
import { appIcon, ensureCustomIcons } from '../store/customIcons';

/** App identity only; decorative doorway illustrations keep their own assets. */
export function HollowMark() {
  onMount(ensureCustomIcons);
  return <img class="hollow-mark" src={appIcon() ?? doorway} alt="" aria-hidden="true" draggable={false}/>;
}
