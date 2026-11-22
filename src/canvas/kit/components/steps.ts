/** `<hc-steps label="Release" current="2">` with one child per step:
 *  `<div label="Spike" when="Oct 4">Optional detail</div>`. A timeline /
 *  stepper. `current` (1-based) marks earlier steps done and that step
 *  current; a child's own `status` (`done`, `current`, `upcoming`,
 *  `blocked`) wins. Each status has its own mark (✓, ●, an empty ring, ✕)
 *  and a screen-reader word, so it never rests on colour; the current step carries
 *  `aria-current="step"`. The step's own children stay as its detail. */
import { attr, h, KitElement, named, part } from '../registry';

const STATUS = {
  done: ['✓', 'Done'],
  current: ['●', 'Current'],
  upcoming: ['', 'Upcoming'],
  blocked: ['✕', 'Blocked'],
} as const;
type Status = keyof typeof STATUS;

export class HcSteps extends KitElement {
  static get observedAttributes(): string[] { return ['label', 'current']; }

  protected render(): void {
    const label = attr(this, 'label');
    named(this, 'list', label);
    const current = Number(this.getAttribute('current')) || 0;
    Array.from(this.children).forEach((step, i) => {
      for (const p of Array.from(step.children)) if (p.hasAttribute('data-hc-part')) p.remove();
      const own = attr(step, 'status').toLowerCase();
      const status: Status = Object.prototype.hasOwnProperty.call(STATUS, own) ? own as Status
        : current ? (i + 1 < current ? 'done' : i + 1 === current ? 'current' : 'upcoming') : 'upcoming';
      step.setAttribute('role', 'listitem');
      step.setAttribute('data-hc-status', status);
      if (status === 'current') step.setAttribute('aria-current', 'step');
      else step.removeAttribute('aria-current');
      const head = part('div', 'hc-step-head');
      head.append(
        h('span', 'hc-step-mark', STATUS[status][0], { 'aria-hidden': 'true' }),
        h('span', 'hc-sr', `${STATUS[status][1]}: `),
        h('span', 'hc-step-label', attr(step, 'label') || `Step ${i + 1}`),
      );
      const when = attr(step, 'when');
      if (when) head.append(h('span', 'hc-step-when', when));
      step.prepend(head);
    });
  }
}
