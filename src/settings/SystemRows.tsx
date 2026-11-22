// Settings, Background work and System rows (L4, X18: extracted from
// pages/Settings.tsx): heartbeat tunables and the build identity (Help's All keys row was cut in Settings PR 6).
import { createResource } from 'solid-js';
import { heartbeatConfig, setHeartbeatConfig } from '../store';
import { api } from '../api';
import type { HeartbeatConfig } from '../types';
import { copyBuildIdentity } from '../appMenu';
import { Mini } from './MergedRow';
import { parseWhole } from './mergedRows';

/// Heartbeat tunables — work-hours window, min spacing, daily budget.
export function HeartbeatConfigRows() {
  const cfg = heartbeatConfig;
  const update = (patch: Partial<HeartbeatConfig>) => setHeartbeatConfig({ ...cfg(), ...patch });
  // A value that isn't a whole number in range is put back, never saved.
  const numInput = (val: number, on: (n: number) => void, min: number, max: number, label: string) => (
    <input
      class="settings-input"
      type="number"
      min={min}
      max={max}
      value={val}
      aria-label={label}
      style={{ width: '4.5rem' }}
      onChange={(e) => { const n = parseWhole(e.currentTarget.value, min, max); if (n !== null) on(n); else e.currentTarget.value = String(val); }}
    />
  );
  return (
    <>
      <div class="settings-row">
        <div class="settings-row-label">
          <div class="settings-row-k">Work hours</div>
          <div class="settings-row-v">Only runs between these local hours.</div>
        </div>
        <div class="settings-actions">
          {numInput(cfg().workHoursStart, (n) => update({ workHoursStart: n }), 0, 23, 'Work hours start')}
          <span class="settings-row-v">to</span>
          {numInput(cfg().workHoursEnd, (n) => update({ workHoursEnd: n }), 1, 24, 'Work hours end')}
        </div>
      </div>
      {/* Spacing and the daily cap share a row (Settings PR 5). */}
      <div class="settings-row">
        <div class="settings-row-label">
          <div class="settings-row-k">Beat spacing and daily limit</div>
          <div class="settings-row-v">Minutes apart, and the most beats in a day.</div>
        </div>
        <div class="settings-actions">
          <Mini label="Minimum minutes between beats">{numInput(cfg().minIntervalMin, (n) => update({ minIntervalMin: n }), 5, 720, 'Minimum minutes between beats')}</Mini>
          <Mini label="Max beats per day">{numInput(cfg().dailyBudget, (n) => update({ dailyBudget: n }), 1, 48, 'Max beats per day')}</Mini>
        </div>
      </div>
    </>
  );
}

export function fmtDuration(ms: number): string {
  const mins = Math.round(ms / 60_000);
  if (mins < 60) return `${mins} minute${mins === 1 ? '' : 's'}`;
  const hrs = mins / 60;
  return hrs === 1 ? '1 hour' : `${hrs} hours`;
}

/// This binary's version, commit and fingerprint, copyable (audit H14), so a
/// report can name the installed build without a terminal.
export function BuildIdentityRow() {
  const [identity] = createResource(() => api.buildIdentity().catch(() => null));
  return (
    <div class="settings-row">
      <div class="settings-row-label">
        <div class="settings-row-k">Build identity</div>
        <div class="settings-row-v mono-v">{identity.loading ? 'Reading…' : identity()?.text ?? 'Unavailable in this build.'}</div>
      </div>
      <button class="btn" disabled={!identity()} onClick={() => void copyBuildIdentity()}>Copy build identity</button>
    </div>
  );
}
