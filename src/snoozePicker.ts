import { formatClock } from './formatTime';
import { localMinute, parseSnoozeTime, snoozePresets } from './conversationOrganization';
import { showPrompt } from './store';

/** Ask when to snooze a conversation: quick times (in an hour, tomorrow
 *  morning, next week) as buttons, plus a date and time picker. Resolves to
 *  an ISO instant, or null when cancelled; `invalid` runs when the typed time
 *  is not a future minute. */
export function pickSnoozeTime(now: number, invalid: () => void): Promise<string | null> {
  const presets = snoozePresets(now);
  return new Promise(resolve => {
    const start = presets.find(p => p.id === 'tomorrow') ?? presets[0];
    void showPrompt({
      title: 'Snooze conversation',
      confirmLabel: 'Snooze',
      input: { type: 'datetime-local', label: 'Or pick a date and time', value: start ? localMinute(Date.parse(start.at)) : '', min: localMinute(now) },
      aside: {
        heading: 'Snooze until', actionLabel: 'Snooze',
        rows: presets.map(p => ({ id: p.id, label: p.label, detail: formatClock(Date.parse(p.at), now) })),
        // A preset resolves first: the dialog's own cancel lands a tick later.
        onAction: id => resolve(presets.find(p => p.id === id)?.at ?? null),
      },
    }).then(value => {
      if (!value) { resolve(null); return; }
      const parsed = parseSnoozeTime(value, Date.now());
      if (!parsed) invalid();
      resolve(parsed);
    });
  });
}
