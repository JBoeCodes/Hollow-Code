import { Show } from 'solid-js';
import { formatRunElapsed } from '../runElapsed';
import { secondClock } from '../store/secondClock';

/** The ticking label: mounted only while a row runs, so the shared second
 *  clock only runs while something does. */
function Elapsed(props: { since: number }) {
  const now = secondClock();
  return <time class="work-row-timer" aria-hidden="true">{formatRunElapsed(now() - props.since)}</time>;
}

/** The live elapsed time of a running sidebar row. Hidden from screen
 *  readers (a ticking label would be read every second); the row's state
 *  word already says Running. */
export function RunTimer(props: { since: number | null }) {
  return <Show when={props.since}>{since => <Elapsed since={since()}/>}</Show>;
}
