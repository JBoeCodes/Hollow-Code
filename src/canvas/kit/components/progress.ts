/** `<hc-progress label="Plan" value="3" max="5" tone="info"></hc-progress>`:
 *  a labelled progress bar with its numbers in text (never the bar alone).
 *  `max` defaults to 100, where the text reads as a percentage. */
import { attr, part, tone, toneMark } from '../registry';
import { TextureElement } from '../textureElement';

function num(v: string | null): number | null {
  if (v === null || v.trim() === '') return null;
  const n = Number(v);
  return Number.isFinite(n) ? n : null;
}

export class HcProgress extends TextureElement {
  static get observedAttributes(): string[] { return ['label', 'value', 'max', 'tone']; }

  protected render(): void {
    this.clearParts();
    const t = tone(this);
    const label = attr(this, 'label');
    const given = num(this.getAttribute('max'));
    const max = given !== null && given > 0 ? given : 100;
    const raw = num(this.getAttribute('value'));
    const value = raw === null ? null : Math.min(Math.max(raw, 0), max);
    const pct = value === null ? 0 : (value / max) * 100;
    const text = raw === null ? '—' : (max === 100 ? `${raw}%` : `${raw} of ${max}`) + (raw !== value ? ' (outside range)' : '');

    const head = part('div', 'hc-progress-head');
    if (t !== 'neutral') head.append(toneMark(t, false));
    if (label) head.append(Object.assign(document.createElement('span'), { className: 'hc-progress-label', textContent: label }));
    head.append(Object.assign(document.createElement('span'), { className: 'hc-progress-text', textContent: text }));

    const attrs: Record<string, string> = { role: 'progressbar', 'aria-valuemin': '0', 'aria-valuemax': String(max) };
    if (label) attrs['aria-label'] = label;
    if (value !== null) {
      attrs['aria-valuenow'] = String(value);
      attrs['aria-valuetext'] = max === 100 ? text : `${text} (${Math.round(pct)}%)`;
    }
    const track = part('div', 'hc-progress-track', null, attrs);
    const fill = document.createElement('div');
    fill.className = 'hc-progress-fill';
    fill.style.setProperty('--hc-pct', String((value ?? 0) / max));
    track.append(fill);
    this.prepend(head, track);
  }
}
