// M5 UI: what the window says about a chat's move (`docs/reference/peers.md`,
// "Moving a chat"). Pure: no IPC, no signals; chatMoveWords.test.ts holds
// them. The store (`chatMoves.ts`), the chat's notice (`ChatMoveNotice.tsx`),
// the lists' moved badge and Settings' Forget read these.
import type { ChatMoveResult, HomeRow, PeerMachinesView } from '../generated/ipc';

/** A paired Mac's name; null when it isn't paired (forgotten, or never). */
export function macName(view: PeerMachinesView | null | undefined, machineId: string): string | null {
  return view?.machines.find(m => m.machineId === machineId)?.name ?? null;
}

/** The Mac Move to runner moves to, by name; null hides the menu item. */
export function moveRunnerName(view: PeerMachinesView | null | undefined): string | null {
  return view?.moveTo ? macName(view, view.moveTo) : null;
}

/** Move to runner's confirm: what happens, said once before it starts. */
export function moveConfirm(title: string, to: string): { title: string; body: string } {
  return {
    title: `Move “${title}” to ${to}?`,
    body: `It keeps going there with its history, and ${to}’s own agent logins pay for its turns. A running turn stops at its next safe point first. Its files and attachments stay on this Mac.`,
  };
}

/** What a move command's answer shows: its sentence, whether it failed,
 *  and whether Open (the chat at its new home) is offered. */
export interface MoveNotice { text: string; failed: boolean; open: boolean }

export function moveNotice(r: ChatMoveResult): MoveNotice {
  const text = r.continued && r.outcome === 'moved' ? `${r.message} Its turn continues there.` : r.message;
  return { text, failed: r.outcome === 'refused', open: r.outcome === 'moved' && !!r.to };
}

/** The chat's own notice for its home record, or null for none. `name`
 *  is the other Mac's name, null when it is no longer paired. */
export interface HomeNotice {
  kind: 'moving' | 'waiting' | 'moved' | 'forgotten' | 'arrived';
  text: string;
  /** Keep it here and Release here (a chat that is moving or waiting). */
  keep: boolean;
  /** Open it at its new home (a moved chat whose Mac is paired). */
  open: boolean;
}

export function homeNotice(home: HomeRow | undefined, name: string | null): HomeNotice | null {
  if (!home) return null;
  const mac = name ?? 'the other Mac';
  switch (home.state) {
    case 'leaving':
      return home.waiting
        ? { kind: 'waiting', keep: true, open: false, text: `Waiting for ${mac} to answer. It runs on this Mac again once ${mac} lets it go, or stays there if it already ran there.` }
        : { kind: 'moving', keep: true, open: false, text: `Moving to ${mac}. It runs on neither Mac until ${mac} answers.` };
    case 'away':
      return name
        ? { kind: 'moved', keep: false, open: true, text: `Moved to ${name}. It keeps going there; this copy is read-only.` }
        : { kind: 'forgotten', keep: false, open: false, text: 'Moved to a Mac you forgot. It runs there; pair that Mac again to open it here.' };
    case 'arrived':
      return { kind: 'arrived', keep: false, open: false, text: `Moved here from ${name ?? 'another Mac'}.` };
    default:
      return null;
  }
}

/** Release here's confirm: what it does, and that the other Mac may keep a copy. */
export function releaseConfirm(name: string | null, chat: string): { title: string; body: string } {
  const mac = name ?? 'the other Mac';
  return {
    title: `Release “${chat}” here?`,
    body: `It runs on this Mac again now, without waiting for ${mac}. If ${mac} already took it, ${mac} keeps its copy too and the two go their own ways. Use this only when ${mac} is gone or won’t answer.`,
  };
}

/** Forget this Mac's confirm: the base sentence, then the chats it releases
 *  and the ones that stay there. `title` names a chat by its key. */
export function forgetBody(name: string, machineId: string, homes: readonly HomeRow[], title: (key: string) => string): string {
  const base = `This Mac stops opening ${name}’s chats and deletes its key. Hollow unpairs there too when it answers. Nothing on ${name} is deleted.`;
  const moving = homes.filter(h => h.peer === machineId && h.state === 'leaving').map(h => `“${title(h.key)}”`);
  const moved = homes.filter(h => h.peer === machineId && h.state === 'away').length;
  const parts = [base];
  if (moving.length) {
    parts.push(`${moving.length === 1 ? 'One chat is' : `${moving.length} chats are`} still moving there: ${list(moving)}. ${moving.length === 1 ? 'It runs' : 'They run'} on this Mac again; ${name} may keep a copy if it took ${moving.length === 1 ? 'it' : 'them'}.`);
  }
  if (moved) {
    parts.push(`${moved === 1 ? 'One chat that moved there stays' : `${moved} chats that moved there stay`} there, shown here as moved to a Mac you forgot.`);
  }
  return parts.join(' ');
}

function list(items: readonly string[]): string {
  const shown = items.slice(0, 3);
  const more = items.length - shown.length;
  return more > 0 ? `${shown.join(', ')} and ${more} more` : shown.length > 1 ? `${shown.slice(0, -1).join(', ')} and ${shown[shown.length - 1]}` : shown[0];
}

/** The moved badge's accessible name: a row that lives on `name` now. */
export function movedLabel(name: string | null): string {
  return name ? `Moved to ${name}` : 'Moved to a Mac you forgot';
}

/** Settings' hint under Move running chats when this Mac sleeps, for the
 *  switch as it stands: which Mac, and whose logins pay. At most 90
 *  characters: a long Mac name is cut to fit. */
export const HINT_MAX = 90;
export function sleepHint(runner: string | null, on: boolean): string {
  if (!runner) return on ? 'On, but no paired Mac is the runner now, so nothing moves.' : 'No paired Mac is the runner now, so nothing moves.';
  const say = (mac: string) => on ? `On: running chats go to ${mac} when this Mac sleeps; its logins pay.` : `Off. When on, running chats go to ${mac}, and its logins pay for them.`;
  const room = HINT_MAX - say('').length;
  return say(runner.length <= room ? runner : `${runner.slice(0, Math.max(1, room - 1)).trimEnd()}…`);
}
