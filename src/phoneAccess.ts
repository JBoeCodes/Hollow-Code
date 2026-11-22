/**
 * Settings, Devices, Companion devices, iPhone app (the phone chat
 * API, 2026-11-04; contract docs/PHONE_CHAT_API.md, backend
 * `phone_access.rs`): the section's words and choices from
 * `phone_access_status`, and the one list of paired devices it shares with
 * the companion devices (the menu-bar app's signed changes and a phone's
 * chats are the same list, each row revoked its own way). Pure, type-only
 * imports, so the node vitest suite can load it.
 */
import type { CompanionDevice, PhoneAccessStatus, PhoneDevice, PhonePush, PhoneServiceForward, PhoneTailscale } from './types';

/** Shown for a companion device's `platform`. */
export const PLATFORM_LABELS: Record<string, string> = {
  macos: 'Mac', ios: 'iPhone', ipados: 'iPad', watchos: 'Watch', web: 'Web',
};

/** The tailnet port Tailscale publishes the phone chat API on. */
export const SERVE_PORT = 8443;

export type PhoneTone = 'ok' | 'warn' | 'off';

/** Tailnet ports `tailscale serve --tcp` forwards straight to the phone chat
 *  API's port in the saved serve config, which Stop publishing removes (none
 *  from a status saved before the field). */
const tcpForwards = (ts: PhoneTailscale): number[] => ts.tcpForwards ?? [];
/** The same from a `tailscale serve` still running in a terminal, and in a
 *  Tailscale service's config: Stop publishing can't reach either (third
 *  review, 2026-11-05), so Settings says how to stop them. */
const foregroundForwards = (ts: PhoneTailscale): number[] => ts.foregroundForwards ?? [];
const serviceForwards = (ts: PhoneTailscale): PhoneServiceForward[] => ts.serviceForwards ?? [];
const rawForwards = (ts: PhoneTailscale): number => tcpForwards(ts).length + foregroundForwards(ts).length + serviceForwards(ts).length;

/** `port 9000`, or `ports 9000, 10000`. */
const ports = (list: number[]) => list.length === 1 ? `port ${list[0]}` : `ports ${list.join(', ')}`;
/** Why a raw TCP forward is a problem. */
const UNCHECKED = "which doesn't say who is calling, so Hollow can't check that it's you";

/** The chip beside the section's heading. */
export function phoneStateLabel(status: PhoneAccessStatus | null, loading: boolean): { label: string; tone: PhoneTone } {
  if (!status) return { label: loading ? 'Checking…' : 'Status unavailable', tone: 'off' };
  if (!status.available) return { label: 'Not available here', tone: 'off' };
  if (status.recordError) return { label: 'Needs attention', tone: 'warn' };
  if (!status.enabled) return { label: 'Off', tone: 'off' };
  if (!status.listening) return { label: "Can't start", tone: 'warn' };
  // Funnel on another app's 8443 doesn't open Hollow (`tailscaleLine` says).
  if (status.tailscale.funnel && status.tailscale.published) return { label: 'Open to the internet', tone: 'warn' };
  if (rawForwards(status.tailscale)) return { label: 'Needs attention', tone: 'warn' };
  if (status.tailscale.published) return { label: 'Ready', tone: 'ok' };
  return { label: 'Not published', tone: 'warn' };
}

/** One line under the switch: what the server and Tailscale are doing, and
 *  a command for the person to run, shown as code after the text. */
export interface PhoneLine { text: string; tone: 'plain' | 'warn'; command?: string }

/** Whether this Mac listens: on, off, or why it can't. */
export function serverLine(status: PhoneAccessStatus): PhoneLine {
  const address = `127.0.0.1:${status.port}`;
  if (!status.enabled) return { text: "Off. Paired iPhones can't reach this Mac until you turn it on.", tone: 'plain' };
  if (status.listening) return { text: `Listening on ${address}, on this Mac only.`, tone: 'plain' };
  return { text: status.bindError ?? `Hollow couldn't start listening on ${address}.`, tone: 'warn' };
}

/** Whether Tailscale publishes it to the tailnet, and what is in the way. */
export function tailscaleLine(status: PhoneAccessStatus): PhoneLine {
  const ts = status.tailscale;
  if (!ts.installed) return { text: "Tailscale isn't installed on this Mac. Install it and sign in, then publish.", tone: 'warn' };
  if (ts.error) return { text: ts.error, tone: 'warn' };
  if (!ts.running) return { text: "Tailscale isn't running or signed in on this Mac.", tone: 'warn' };
  if (ts.funnel && ts.published) {
    return { text: `Port ${SERVE_PORT} is open to the internet with Tailscale Funnel. Hollow is for your tailnet only: choose Stop publishing, then check that Funnel is off in Tailscale.`, tone: 'warn' };
  }
  // A raw TCP forward passes on nothing of who is calling (second review,
  // 2026-11-04), so Hollow would take any tailnet device's requests for
  // this Mac's own. Here the saved ones; `forwardLines` the rest.
  const forwards = tcpForwards(ts);
  if (forwards.length) {
    return { text: `Tailscale forwards tailnet ${ports(forwards)} straight to Hollow with --tcp, ${UNCHECKED}. Choose Stop publishing to remove ${forwards.length === 1 ? 'it' : 'them'}.`, tone: 'warn' };
  }
  // Another app's 8443, Funnel or not: Stop publishing never touches it
  // (the review of the third review's fixes, 2026-11-05).
  if (ts.other && ts.funnel) return { text: `Tailscale already serves ${ts.other} on port ${SERVE_PORT}, open to the internet with Tailscale Funnel. Hollow won't replace or change it; stop that first.`, tone: 'warn' };
  if (ts.other) return { text: `Tailscale already serves ${ts.other} on port ${SERVE_PORT}. Hollow won't replace it; stop that first.`, tone: 'warn' };
  if (ts.funnel) return { text: `Tailscale Funnel is on for port ${SERVE_PORT}, which doesn't serve Hollow. Hollow won't publish there until Funnel is off for that port in Tailscale.`, tone: 'warn' };
  if (ts.published) {
    const where = status.baseUrl ? ` at ${status.baseUrl}` : '';
    return { text: `Published to your tailnet${where}. Only your own Tailscale account can use it.`, tone: 'plain' };
  }
  return { text: 'Not published on Tailscale yet. Your iPhone reaches this Mac once it is.', tone: 'plain' };
}

/** The raw TCP forwards Stop publishing can't reach (third review,
 *  2026-11-05: `tailscale serve --tcp=<port> off` edits only the saved
 *  config), each saying how to stop it: one line for a `tailscale serve`
 *  still running in a terminal, and one per service forward with the
 *  command that removes it, or, for a service whose name Hollow won't put
 *  in a command (`stop` is null), the same in words without the name. */
export function forwardLines(status: PhoneAccessStatus): PhoneLine[] {
  const ts = status.tailscale;
  const lines: PhoneLine[] = [];
  const foreground = foregroundForwards(ts);
  if (foreground.length) {
    lines.push({
      text: `A tailscale serve command still running in a terminal (started without --bg) forwards tailnet ${ports(foreground)} straight to Hollow with --tcp, ${UNCHECKED}. Stop publishing can't end it: press Control-C in that terminal, then choose Check again.`,
      tone: 'warn',
    });
  }
  for (const forward of serviceForwards(ts)) {
    if (forward.stop) {
      lines.push({
        text: `Tailscale service ${forward.service} forwards tailnet port ${forward.port} straight to Hollow with --tcp, ${UNCHECKED}. Stop publishing can't remove it: run this in a terminal, then choose Check again:`,
        tone: 'warn',
        command: forward.stop,
      });
      continue;
    }
    lines.push({
      text: `A Tailscale service with a name Hollow won't put in a command forwards tailnet port ${forward.port} straight to Hollow with --tcp, ${UNCHECKED}. Stop publishing can't remove it: in a terminal, find the service with tailscale serve status, remove its forward with tailscale serve, giving --service its name, --tcp=${forward.port} and off, then choose Check again.`,
      tone: 'warn',
    });
  }
  return lines;
}

/** The Tailscale button: Publish on Tailscale or Stop publishing, and why
 *  it can't be pressed now. Null when the section can't be used here. Stop
 *  publishing only for what it removes (8443 serving Hollow, Funnel on it
 *  included, and a saved forward): another app's 8443 and a forward a
 *  terminal or a service holds get no button, since it never removes them
 *  (`tailscaleLine` and `forwardLines` say what to do). */
export interface PublishChoice { kind: 'publish' | 'unpublish'; label: string; disabled: boolean; why: string | null }

export function publishChoice(status: PhoneAccessStatus | null): PublishChoice | null {
  if (!status?.available) return null;
  const ts = status.tailscale;
  if (ts.published || tcpForwards(ts).length) return { kind: 'unpublish', label: 'Stop publishing', disabled: false, why: null };
  const publish = (why: string | null): PublishChoice => ({ kind: 'publish', label: 'Publish on Tailscale', disabled: why !== null, why });
  if (!ts.installed) return publish('Install Tailscale first.');
  if (!ts.running) return publish('Start Tailscale and sign in first.');
  if (ts.other) return publish(`Port ${SERVE_PORT} serves something else.`);
  if (ts.funnel) return publish(`Tailscale Funnel is on for port ${SERVE_PORT}.`);
  return publish(null);
}

/** Pair an iPhone: why it can't be pressed now, or null when it can. */
export function pairBlocked(status: PhoneAccessStatus | null): string | null {
  if (!status?.available) return status?.unavailableReason ?? 'The iPhone app is unavailable.';
  if (!status.enabled || !status.listening) return 'Turn on the iPhone app first.';
  return null;
}

/** A pairing code read out one character at a time (`K7M3-Q9TX` → `K 7 M 3 Q 9 T X`). */
export function spelledCode(code: string): string {
  return code.replace(/[\s-]/g, '').split('').join(' ');
}

/** The QR code as an image address, so it shows without inserting markup.
 *  Only an SVG document without script or outside references. */
export function qrImageSrc(svg: string | null | undefined): string | null {
  const text = svg?.trim() ?? '';
  if (!text || !/^(<\?xml[^>]*\?>\s*)?<svg[\s>]/i.test(text) || !/<\/svg>$/i.test(text)) return null;
  if (/<script|<foreignObject|\bon\w+\s*=|(?:href|src)\s*=\s*["']?\s*(?!#)[a-z]/i.test(text)) return null;
  return `data:image/svg+xml;charset=utf-8,${encodeURIComponent(text)}`;
}

/** One row of Companion devices' list: a device that sends signed changes
 *  (the menu-bar app) or an iPhone paired with the iPhone app. */
export interface DeviceRow {
  key: string;
  kind: 'companion' | 'phone';
  deviceId: string;
  name: string;
  /** What it is: `Mac`, `iPhone app · iPhone 16 Pro`. */
  what: string;
  pairedAt: string;
  /** The last thing it did and when. */
  last: { label: string; at: string } | null;
}

const when = (at: string) => { const t = Date.parse(at); return Number.isNaN(t) ? Number.POSITIVE_INFINITY : t; };

/** Every paired device in one list, oldest pairing first. */
export function deviceRows(companion: readonly CompanionDevice[], phones: readonly PhoneDevice[]): DeviceRow[] {
  const rows: DeviceRow[] = [
    ...companion.map((d): DeviceRow => ({
      key: `companion:${d.deviceId}`, kind: 'companion', deviceId: d.deviceId, name: d.name,
      what: PLATFORM_LABELS[d.platform] ?? d.platform, pairedAt: d.pairedAt,
      last: d.lastOpAt ? { label: 'last change', at: d.lastOpAt } : null,
    })),
    ...phones.map((d): DeviceRow => ({
      key: `phone:${d.deviceId}`, kind: 'phone', deviceId: d.deviceId, name: d.name,
      what: d.model?.trim() ? `iPhone app · ${d.model.trim()}` : 'iPhone app', pairedAt: d.pairedAt,
      last: d.lastSeenAt ? { label: 'last seen', at: d.lastSeenAt } : null,
    })),
  ];
  return rows
    .map((row, i) => ({ row, i }))
    .sort((a, b) => when(a.row.pairedAt) - when(b.row.pairedAt) || a.i - b.i)
    .map(({ row }) => row);
}

/** What Revoke asks before it removes the device. */
export function revokeConfirm(row: DeviceRow): { title: string; body: string } {
  if (row.kind === 'phone') {
    return {
      title: `Revoke ${row.name}?`,
      body: 'It is signed out of Hollow at once: its next request is refused and its open chats stop updating. You can pair it again later with a new code.',
    };
  }
  return {
    title: `Revoke ${row.name}?`,
    body: 'Its key is deleted, so changes it sends from now on are refused. You can pair it again later with a new code.',
  };
}

/** v1.1 push notifications: Settings' one-word status ("Push: ready / no
 *  key / error"). */
export function pushStateLabel(push: PhonePush | null | undefined): { label: string; tone: PhoneTone } {
  if (!push) return { label: 'Push: unavailable', tone: 'off' };
  if (push.state === 'ready') return { label: 'Push: ready', tone: 'ok' };
  if (push.state === 'error') return { label: 'Push: error', tone: 'warn' };
  return { label: 'Push: no key', tone: 'off' };
}

/** What the push block says under its status: the key, and which phones
 *  get notifications. The last send and the last error carry their own
 *  times. */
export function pushLines(push: PhonePush | null | undefined): string[] {
  if (!push) return [];
  const lines: string[] = [];
  if (push.state === 'ready' && push.keyId) lines.push(`Key ${push.keyId}, team ${push.teamId ?? 'not set'}, kept in this Mac's Keychain.`);
  if (push.state === 'noKey') lines.push('Import the APNs key (.p8) from your Apple developer account to turn notifications on.');
  lines.push(push.phones === 0
    ? 'No paired iPhone has turned on notifications yet.'
    : `${push.phones} paired iPhone${push.phones === 1 ? '' : 's'} get${push.phones === 1 ? 's' : ''} notifications.`);
  return lines;
}

/** The Key ID in Apple's file name for a key, `AuthKey_<Key ID>.p8`
 *  (`push::key_id_from_file_name` in Rust), or null. */
export function keyIdFromPath(path: string | null | undefined): string | null {
  const match = /AuthKey_([A-Za-z0-9]{10})\.p8$/.exec(path ?? '');
  return match ? match[1].toUpperCase() : null;
}

/** Apple's developer team for the iPhone app: the import's default. */
export const PUSH_TEAM_ID = 'TEAM000000';
