/** Usage graphics use ledger buckets, never inferred prices or fabricated zero days. */
import type { UsageBucket, UsageDay } from './types';
import { calendarDay, gaugeValue } from './canvas/kit/instruments';
import { formatUsd } from './usageFormat';
export type CostBasis = 'reportedCostUsd' | 'estimatedCostUsd';
export const positiveCost = (value: number | null | undefined): number | null => value != null && Number.isFinite(value) && value > 0 ? value : null;
export function coverageWords(b: UsageBucket): string {
  return [b.partial && `${b.partial} partial`, b.unknown && `${b.unknown} unknown`, b.unpriced && `${b.unpriced} unpriced`, b.gpuTime && `${b.gpuTime} billed by GPU time`, b.inProgress && `${b.inProgress} in progress`, b.notStarted && `${b.notStarted} not started`].filter(Boolean).join(' · ') || 'No incomplete runs recorded';
}
/** The agent split's amount: a self-hosted model's bucket is billed by GPU
 *  time (Q8), never "no amount" or $0. */
export function agentAmount(b: UsageBucket, basis: CostBasis): string {
  return formatUsd(b[basis]) ?? (b.gpuTime ? 'Billed by GPU time' : 'No amount recorded');
}
export function usageSeries(days: UsageDay[], basis: CostBasis) {
  const agents = new Map<string, string>();
  days.forEach(d=>d.byAgent.forEach(b=>agents.set(b.key,b.label)));
  return [...agents].sort(([a],[b])=>a.localeCompare(b)).map(([key,label])=>({key,label,values:days.map(d=>positiveCost(d.byAgent.find(b=>b.key===key)?.[basis]))}));
}
/** Step-area silhouettes: full daily cells avoid implying interpolated spend between days. */
export function stackedPaths(days: UsageDay[], basis: CostBasis) {
  const series=usageSeries(days,basis);
  const totals=days.map((_,i)=>series.reduce((n,s)=>n+(s.values[i]??0),0));
  const max=Math.max(0,...totals);
  const dates=days.map(d=>calendarDay(d.date));
  const known=dates.filter((d):d is number=>d!==null);
  const first=known.length?Math.min(...known):0, last=known.length?Math.max(...known):first;
  const width=600 / Math.max(1,last-first+1);
  const x=(i:number)=>40+((dates[i]??first)-first)*width;
  const y=(v:number)=>200-v/Math.max(1e-12,max)*168;
  const base=days.map(()=>0);
  const layers=series.map((s,index)=>{
    const paths:string[]=[];let top:string[]=[],bottom:string[]=[];let previous:number|null=null;
    const flush=()=>{if(top.length)paths.push(`M${top.join(' L')} L${bottom.reverse().join(' L')} Z`);top=[];bottom=[];};
    s.values.forEach((v,i)=>{
      const date=dates[i];
      if(v===null||date===null){flush();previous=null;return;}
      if(previous!==null&&date!==previous+1)flush();
      top.push(`${x(i)},${y(base[i]+v)}`,`${x(i)+width},${y(base[i]+v)}`);
      bottom.push(`${x(i)},${y(base[i])}`,`${x(i)+width},${y(base[i])}`);
      base[i]+=v;previous=date;
    });flush();return {...s,index,paths};
  });
  return {max,layers,x,width};
}
/** Same semicircle model as hc-gauge. Unknown has no value arc, never a 0% claim. */
export function budgetGauge(percent:number|null|undefined) {
  if(percent==null||!Number.isFinite(percent))return null;
  const g=gaugeValue(String(percent),'0','100');const fraction=g.fraction!;
  const a=Math.PI*(1-fraction);
  return {fraction,label:`${Math.round(percent)}%`,path:`M24,112 A96,96 0 0 1 ${120+96*Math.cos(a)},${112-96*Math.sin(a)}`};
}
