/**
 * Settings, Usage, Limits (Settings PR 8): one meter with its marks. The
 * shared .meter primitive, plus a line at each mark: the Move on at share on
 * a Claude window, a budget's warn percent and, for a hard stop, its limit.
 */
import { For, Show } from 'solid-js';
import '../styles/usage-limits.css';

export interface MeterMark { at: number; kind: 'move' | 'warn' | 'stop'; title: string }

export function UsageMeter(props: {
  label: string;
  /** 0 to 1. */
  fill: number;
  level: string;
  value: string;
  meta?: string | null;
  marks?: readonly MeterMark[];
  valueText?: string;
}) {
  return (
    <div class="usage-limit" data-level={props.level}>
      <div class="usage-limit-row">
        <span>{props.label}</span>
        <span class="usage-limit-value">{props.value}</span>
      </div>
      <div class="usage-meter">
        <div class="meter" role="meter" aria-label={props.label} aria-valuemin={0} aria-valuemax={100}
          aria-valuenow={Math.round(props.fill * 100)} aria-valuetext={props.valueText ?? props.value}>
          <span class="meter-track"><span class="meter-fill" style={{ '--pct': props.fill }} /></span>
        </div>
        <For each={props.marks ?? []}>
          {mark => <i class="usage-mark" data-mark={mark.kind} title={mark.title} style={{ '--at': Math.min(Math.max(mark.at, 0), 1) }} aria-hidden="true" />}
        </For>
      </div>
      <Show when={props.meta}>{meta => <span class="usage-limit-meta">{meta()}</span>}</Show>
    </div>
  );
}
