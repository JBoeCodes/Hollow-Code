// The card's Servers and Automations sections, as pure functions (parity 5.5):
// which automations belong to this chat and the words for their rows and the
// two section counts. CardAutomations.tsx and ServicesPanel.tsx render them;
// cardAutomationsModel.test.ts holds them. Local and provider-neutral.
import type { Workflow } from './generated/ipc';
import type { ServiceView } from './workspaceTypes';
import { describeSchedule } from './workflowSchedule';

/** The automations bound to this chat (the `in_chat` key), newest first by next run. */
export function chatAutomations(list: readonly Workflow[] | null | undefined, chatKey: string | null | undefined): Workflow[] {
  if (!chatKey) return [];
  return (list ?? []).filter(w => w.inChat === chatKey).sort((a, b) => (a.nextRunAt ?? '9999').localeCompare(b.nextRunAt ?? '9999'));
}

/** The Automations section's count: "None", "1 here" or "3 here". */
export function automationsCount(n: number): string {
  return n === 0 ? 'None' : `${n} here`;
}

/** One row's second line: the schedule, then how the last run went. */
export function automationWords(w: Pick<Workflow, 'schedule' | 'lastStatus' | 'lastRunAt' | 'nextRunAt'>, now: number = Date.now()): string {
  const parts = [describeSchedule(w.schedule)];
  if (w.lastStatus === 'error') parts.push('last run failed');
  else if (w.lastStatus === 'ok' || w.lastStatus === 'success') parts.push('last run ok');
  const next = w.nextRunAt ? Date.parse(w.nextRunAt) : NaN;
  if (Number.isFinite(next) && next > now) parts.push(`next ${new Date(next).toLocaleString([], { weekday: 'short', hour: 'numeric', minute: '2-digit' })}`);
  return parts.join(', ');
}

/** The Servers section's count: "None running", "1 running" or "2 running, 1 exited". */
export function serversCount(services: readonly Pick<ServiceView, 'status'>[]): string {
  const running = services.filter(s => s.status === 'running').length;
  const exited = services.length - running;
  if (!services.length) return 'None running';
  return [running ? `${running} running` : '', exited ? `${exited} exited` : ''].filter(Boolean).join(', ');
}

/** What Add an automation drafts: the person finishes the sentence and sends it. */
export const ADD_AUTOMATION_PROMPT = 'Create an automation that runs in this chat (say what it should do and when): ';
