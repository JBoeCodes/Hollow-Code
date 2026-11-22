import { For, Show, createMemo, createSignal, createUniqueId } from 'solid-js';
import type { UsageSummary } from '../types';
import { bucketCost, formatUsd } from '../usageFormat';
import { agentAmount, coverageWords, positiveCost, stackedPaths, type CostBasis } from '../usageInstruments';
import { observeChatMotion } from './ChatMotion';
import '../styles/usage-instruments.css';

/** Pure summary view, mounted only under Settings, Usage. No polling or canvas frame. */
export default function UsageInstruments(props:{summary:UsageSummary}) {
  const id=createUniqueId();
  const [basis,setBasis]=createSignal<CostBasis>('estimatedCostUsd');
  const [selected,setSelected]=createSignal<string|null>(null);
  const days=()=>props.summary.daily??[];
  const drawing=createMemo(()=>stackedPaths(days(),basis()));
  const day=()=>days().find(d=>d.date===selected())??days().at(-1);
  const basisLabel=()=>basis()==='reportedCostUsd'?'Reported cost':'Estimated list-price cost';
  const nightAgents=createMemo(()=>[...new Map(days().flatMap(d=>d.nightByAgent.map(b=>[b.key,b.label] as const))).entries()]);
  const agentMax=()=>Math.max(1e-12,...props.summary.byAgent.map(b=>positiveCost(b[basis()])??0));
  return <section class="usage-instruments" aria-labelledby={`${id}-title`} ref={observeChatMotion} data-motion-visible="false" data-motion-static={typeof IntersectionObserver === 'undefined' ? 'true' : undefined}>
    <header class="usage-instruments-head"><div><p class="usage-eyebrow">The shape of your work</p><h3 id={`${id}-title`}>Usage by day</h3></div>
      <div class="seg-toggle" role="group" aria-label="Cost basis">
        <button class="seg-toggle-btn" classList={{active:basis()==='estimatedCostUsd'}} aria-pressed={basis()==='estimatedCostUsd'} onClick={()=>setBasis('estimatedCostUsd')}>Estimated</button>
        <button class="seg-toggle-btn" classList={{active:basis()==='reportedCostUsd'}} aria-pressed={basis()==='reportedCostUsd'} onClick={()=>setBasis('reportedCostUsd')}>Reported</button>
      </div>
    </header>
    <p class="usage-visual-note">{basisLabel()} · USD · local recording day. Estimates are not a bill. Gaps mean no amount recorded, not free work.</p>
    <div class="usage-visual-grid">
      <div class="usage-area-wrap">
        <Show when={drawing().max>0} fallback={<div class="usage-chart-empty">No {basis()==='reportedCostUsd'?'reported':'estimated'} amounts recorded in this period.</div>}>
          <div class="usage-scale" aria-hidden="true"><span>Known daily amount</span><span>Peak {formatUsd(drawing().max)}</span></div>
          <svg class="usage-area" viewBox="36 20 608 185" aria-hidden="true">
            <For each={[0,0.5,1]}>{f=><g><line x1="40" x2="640" y1={200-f*168} y2={200-f*168}/></g>}</For>
            <g class="usage-entrance"><For each={drawing().layers}>{layer=><g class={`usage-agent-color agent-${layer.index%5}`}><For each={layer.paths}>{path=><path d={path}/>}</For></g>}</For></g>

          </svg>
          <div class="usage-axis" aria-hidden="true"><span>{days()[0]?.date}</span><span>{days().length>1?days().at(-1)?.date:''}</span></div>
        </Show>
        <div class="usage-day-picks" role="group" aria-label="Inspect a recording day"><For each={days()}>{d=><button type="button" classList={{active:day()?.date===d.date}} aria-pressed={day()?.date===d.date} aria-label={`Inspect ${d.date}`} onClick={()=>setSelected(d.date)} title={d.date}>{d.date.slice(5)}</button>}</For></div>
        <div class="usage-day-detail" role="status" aria-live="polite" aria-atomic="true"><strong>{day()?.date??'No recording days'}</strong><span>{day()?bucketCost(day()!.total):'No daily records available.'}</span><span>{day()?coverageWords(day()!.total):'Missing coverage stays unknown.'}</span></div>
      </div>
      <div class="usage-agent-split"><h4>Agent split</h4><p class="usage-visual-note">{basisLabel()}</p><For each={props.summary.byAgent}>{b=>{
        const index=()=>[...new Set([...drawing().layers.map(s=>s.key),...props.summary.byAgent.map(a=>a.key)])].indexOf(b.key);
        return <div class={`usage-agent-row usage-agent-color agent-${Math.max(0,index())%5}`}><strong><i aria-hidden="true"/>{b.label}</strong><span>{agentAmount(b, basis())}</span><div class="usage-agent-track" aria-hidden="true"><Show when={positiveCost(b[basis()])!==null}><i style={{width:`${(positiveCost(b[basis()])!)/agentMax()*100}%`}}/></Show></div><small>{coverageWords(b)}</small></div>;
      }}</For></div>
    </div>
    <div class="usage-night-band"><h4>Night queue · direct runs</h4><p class="usage-visual-note">Recorded invocations, including failover attempts, linked to queued turns and grouped by recording day and agent. Excludes descendants and manually started lanes; this is not the overnight budget total.</p>
      <Show when={!props.summary.nightAttributionError} fallback={<p class="settings-error">Night attribution could not be read.</p>}>
        <Show when={nightAgents().length} fallback={<p class="usage-visual-note">No direct night-queue links found in these records. Other overnight work is not classified.</p>}>
          <div class="usage-night-lane usage-night-dates" aria-hidden="true"><strong>Day</strong><div class="usage-night-cells"><For each={days()}>{d=><span title={d.date}>{d.date.slice(5)}</span>}</For></div></div>
          <For each={nightAgents()}>{([key,label])=><div class="usage-night-lane" aria-hidden="true"><strong>{label}</strong><div class="usage-night-cells usage-entrance"><For each={days()}>{d=>{
            const b=()=>d.nightByAgent.find(b=>b.key===key);
            return <span classList={{recorded:!!b()}} title={`${d.date}: ${b()?`${b()!.runs} direct runs · ${bucketCost(b()!)} · ${coverageWords(b()!)}`:'No direct link'}`} aria-hidden="true">{b()?.runs??'·'}</span>;
          }}</For></div></div>}</For>
        </Show>
      </Show>
    </div>
    <Show when={props.summary.undatedRuns>0}><p class="usage-visual-note">{props.summary.undatedRuns} runs have unreadable dates; they remain in totals and the agent split but are absent from this chart.</p></Show>
    <details class="usage-data"><summary>Daily values and night-queue coverage</summary><div class="usage-data-scroll" role="region" aria-label="Daily usage values" tabindex="0"><table class="usage-table"><caption>Recorded daily amounts; reported and estimated kept separate</caption><thead><tr><th scope="col">Day / agent</th><th scope="col">Cost</th><th scope="col">Coverage</th><th scope="col">Direct night runs</th></tr></thead><tbody><For each={days()}>{d=><For each={d.byAgent}>{b=><tr><th scope="row">{d.date} · {b.label}</th><td>{bucketCost(b)}</td><td>{b.runs} runs · {coverageWords(b)}</td><td>{props.summary.nightAttributionError?'Unavailable':d.nightByAgent.find(n=>n.key===b.key)?`${d.nightByAgent.find(n=>n.key===b.key)!.runs} · ${bucketCost(d.nightByAgent.find(n=>n.key===b.key)!)} · ${coverageWords(d.nightByAgent.find(n=>n.key===b.key)!)}`:'No direct link'}</td></tr>}</For>}</For></tbody></table></div></details>
  </section>;
}
