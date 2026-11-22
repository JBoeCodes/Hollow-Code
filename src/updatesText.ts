import type { UpdateLastResult } from './generated/ipc';

export function updateResultText(result: UpdateLastResult): string {
  switch (result.kind) {
    case 'updated': return `Updated to ${result.local ? `the local build of ${result.local.branch} (${result.version})` : result.version}${result.notes ? `\n${result.notes}` : ''}`;
    case 'wentBack':
      // Row U6: the person's own Go back (manual) has no reason to give.
      if (result.manual) return `Went back to ${result.restored ?? 'the previous version'}. Skipping ${result.attempted}.${result.dataRestored ? ' Your chats are as they were before the update; newer ones are kept aside.' : ''}`;
      return `Went back to ${result.restored ?? 'the previous version'} because ${result.reason}`;
    case 'notWentBack': return `Didn't go back to ${result.target}: ${result.reason ?? 'No reason was reported.'}`;
    case 'notInstalled': return `Update ${result.attempted} was not installed: ${result.reason ?? 'No reason was reported.'}`;
  }
}

export function lastCheckedText(at: string | null | undefined, now = Date.now()): string {
  if (!at) return 'Last checked never';
  const checked = Date.parse(at);
  if (!Number.isFinite(checked)) return 'Last checked never';
  const minutes = Math.max(0, Math.floor((now - checked) / 60_000));
  if (minutes < 1) return 'Last checked just now';
  if (minutes < 60) return `Last checked ${minutes} minute${minutes === 1 ? '' : 's'} ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `Last checked ${hours} hour${hours === 1 ? '' : 's'} ago`;
  const days = Math.floor(hours / 24);
  return `Last checked ${days} day${days === 1 ? '' : 's'} ago`;
}
