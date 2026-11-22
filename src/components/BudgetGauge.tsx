import { Show } from 'solid-js';
import { budgetGauge } from '../usageInstruments';
import { observeChatMotion } from './ChatMotion';
import '../styles/usage-instruments.css';
/** The adjacent budget words expose exact amounts, state and enforcement. */
export function BudgetGauge(props:{percent:number|null;unavailable?:boolean}) {
  const g=()=>props.unavailable?null:budgetGauge(props.percent);
  return <div class="budget-gauge" ref={observeChatMotion} data-motion-visible="false" data-motion-static={typeof IntersectionObserver === 'undefined' ? 'true' : undefined} aria-hidden="true"><svg viewBox="0 0 240 152"><path class="budget-gauge-track" d="M24,112 A96,96 0 0 1 216,112"/><Show when={g()&&g()!.fraction>0}><path class="budget-gauge-value usage-entrance" d={g()!.path}/></Show><text x="120" y="107" text-anchor="middle">{g()?.label??'Unknown'}</text></svg></div>;
}
