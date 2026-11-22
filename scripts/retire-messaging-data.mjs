import { createHash, randomUUID } from 'node:crypto';
import { existsSync, lstatSync, mkdirSync, readFileSync, readdirSync, renameSync, unlinkSync, writeFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { homedir } from 'node:os';
import { fileURLToPath } from 'node:url';
import { assertHollowStopped, resolveVaultFromConfig } from './archive-message-notes.mjs';

const sha = bytes => createHash('sha256').update(bytes).digest('hex');
const normalize = value => String(value ?? '').trim().toLowerCase();
const messagePath = value => /^(?:emails|teams|slack)(?:\/|$)/.test(normalize(value).replaceAll('\\', '/').replace(/^vault:/, '').replace(/^(?:\/|\.\/)+/, ''));
const sourcePaths = value => ['sourcePath', 'sourceVaultPath', 'replyToVaultPath'].some(key => messagePath(value?.[key]));
export function isMessagingReview(item) {
  return ['heartbeat_send', 'reply_draft', 'meeting_followup'].includes(normalize(item?.kind))
    || normalize(item?.risk) === 'external_send'
    || ['inbox', 'email', 'teams', 'slack'].includes(normalize(item?.source))
    || sourcePaths(item)
    || sourcePaths(item?.payload);
}
function retiredReference(value, removed) {
  if (Array.isArray(value)) return value.some(item => retiredReference(item, removed));
  if (value && typeof value === 'object') return Object.values(value).some(item => retiredReference(item, removed));
  if (typeof value !== 'string') return false;
  const text = normalize(value).replaceAll('\\', '/');
  return ['emails/', 'teams/', 'slack/', 'mailto:', 'outlook.office.com/mail',
    'outlook.office365.com/mail', 'teams.microsoft.com/l/chat', 'slack.com/archives/',
    'drafts/heartbeat/reply-', 'drafts/heartbeat/reply_', 'drafts/heartbeat/email-',
    'drafts/heartbeat/email_', 'drafts/heartbeat/teams-', 'drafts/heartbeat/slack-',
    'heartbeat_send', 'reply_draft', 'meeting_followup'].some(marker => text.includes(marker))
    || [...removed].some(path => text.includes(path.toLowerCase()));
}
export function isMessageDraft(path, text) {
  const head = text.slice(0, 1600);
  return /^(?:kind|draft_type):\s*["']?(?:email|teams|slack)/im.test(head)
    || /^(?:type):\s*["']?draft-reply/im.test(head)
    || /\/(?:reply-|email-)/i.test(path)
    || (/^#?\s*Draft(?: reply|:)/im.test(head) && /(?:\*\*)?To:/i.test(head))
    || /^Hi [^\n]+,\s*\n/.test(head);
}
function safeFile(root, rel) {
  const path = join(root, rel);
  let current = path;
  while (true) {
    if (existsSync(current) && lstatSync(current).isSymbolicLink()) throw new Error(`Symlink refused: ${rel}`);
    const parent = dirname(current);
    if (parent === current) break;
    current = parent;
  }
  return path;
}
function walk(root, prefix) {
  const path = safeFile(root, prefix);
  if (!existsSync(path)) return [];
  return readdirSync(path).flatMap(name => {
    const rel = `${prefix}/${name}`;
    const p = safeFile(root, rel);
    return lstatSync(p).isDirectory() ? walk(root, rel) : [rel];
  });
}
export function planRetirement(vault) {
  const writes = [];
  const read = path => {
    const p = safeFile(vault, path);
    return existsSync(p) ? readFileSync(p) : null;
  };
  // A single captured buffer drives selection, transformation, and the apply
  // concurrency check. Never reread a newer file as the base for older output.
  const add = (path, before, after, reason) => {
    if (before === null) return;
    if (after !== null && before.equals(Buffer.from(after))) return;
    writes.push({ path, before, after: after === null ? null : Buffer.from(after), reason });
  };
  for (const path of ['.review-queue.json', '.review-queue.v1.json']) {
    const before = read(path);
    if (before === null) continue;
    const value = JSON.parse(before.toString('utf8'));
    const items = Array.isArray(value) ? value : value.items;
    if (!Array.isArray(items)) throw new Error(`Unexpected queue schema: ${path}`);
    const kept = items.filter(item => !isMessagingReview(item));
    if (kept.length !== items.length) add(path, before, JSON.stringify(Array.isArray(value) ? kept : { ...value, items: kept }, null, 2) + '\n', `${items.length - kept.length} message-related review records`);
  }
  add('.heartbeat-actions.jsonl', read('.heartbeat-actions.jsonl'), null, 'retired outbound draft queue');
  for (const path of walk(vault, 'drafts')) {
    if (!path.endsWith('.md')) continue;
    const before = read(path);
    if (before !== null && isMessageDraft(path, before.toString('utf8'))) add(path, before, null, 'message draft');
  }
  for (const folder of ['_signature', '.hollow-attachments/email']) {
    for (const path of walk(vault, folder)) add(path, read(path), null, 'retired email asset');
  }
  const removed = new Set(writes.filter(x => x.after === null).map(x => x.path));
  for (const path of ['.heartbeat-tasks.jsonl', '.heartbeat-log.jsonl', '.notifications.jsonl']) {
    const before = read(path);
    if (before === null) continue;
    const rows = before.toString('utf8').split('\n').filter(Boolean).map(line => JSON.parse(line));
    const kept = rows.filter(row => {
      if (sourcePaths(row) || sourcePaths(row.payload) || retiredReference(row, removed)) return false;
      if (path === '.heartbeat-log.jsonl' && (Array.isArray(row.proposedSends) ? row.proposedSends.length > 0 : Number(row.proposedSends ?? 0) > 0)) return false;
      if (path === '.notifications.jsonl' && /(?:email|slack|teams|inbox|drafted repl)/i.test(`${row.title ?? ''} ${row.body ?? ''}`)) return false;
      return true;
    });
    if (kept.length !== rows.length) add(path, before, kept.map(x => JSON.stringify(x)).join('\n') + (kept.length ? '\n' : ''), `${rows.length - kept.length} retired message activity records`);
  }
  return writes;
}
export function applyRetirement(vault, plan, recoveryRoot, assertStopped = assertHollowStopped) {
  assertStopped();
  const archive = join(resolve(recoveryRoot), `${new Date().toISOString().replaceAll(':', '-')}-${randomUUID().slice(0, 8)}`);
  if (archive.startsWith(resolve(vault) + '/')) throw new Error('Recovery must be outside the vault');
  safeFile(archive, 'manifest.json');
  mkdirSync(archive, { recursive: true, mode: 0o700 });
  const manifest = { vault: resolve(vault), status: 'prepared', changes: plan.map(x => ({ path: x.path, reason: x.reason, beforeSha256: sha(x.before), afterSha256: x.after === null ? null : sha(x.after) })) };
  const manifestPath = join(archive, 'manifest.json');
  writeFileSync(manifestPath, JSON.stringify(manifest, null, 2) + '\n', { mode: 0o600, flag: 'wx' });
  const completed = [];
  try {
    // Save every original before changing any live file.
    for (const item of plan) {
      const live = safeFile(vault, item.path);
      if (sha(readFileSync(live)) !== sha(item.before)) throw new Error(`Concurrent change: ${item.path}`);
      const backup = join(archive, 'before', item.path);
      mkdirSync(dirname(backup), { recursive: true, mode: 0o700 });
      writeFileSync(backup, item.before, { mode: lstatSync(live).mode & 0o777, flag: 'wx' });
      if (sha(readFileSync(backup)) !== sha(item.before)) throw new Error('Recovery verification failed');
    }
    for (const item of plan) {
      const live = safeFile(vault, item.path);
      if (sha(readFileSync(live)) !== sha(item.before)) throw new Error(`Concurrent change: ${item.path}`);
      if (item.after === null) unlinkSync(live);
      else {
        const temporary = `${live}.retiring-${randomUUID()}`;
        writeFileSync(temporary, item.after, { mode: lstatSync(live).mode & 0o777, flag: 'wx' });
        renameSync(temporary, live);
      }
      completed.push(item);
      if (item.after !== null && sha(readFileSync(live)) !== sha(item.after)) throw new Error('Publication verification failed');
    }
    manifest.status = 'complete';
  } catch (error) {
    const rollback = { restoredPaths: [], preservedConcurrentPaths: [], errors: [] };
    for (const item of completed.reverse()) {
      try {
        const live = safeFile(vault, item.path);
        const unchanged = item.after === null ? !existsSync(live) : existsSync(live) && sha(readFileSync(live)) === sha(item.after);
        if (!unchanged) {
          rollback.preservedConcurrentPaths.push(item.path);
          continue;
        }
        writeFileSync(live, item.before);
        if (sha(readFileSync(live)) !== sha(item.before)) throw new Error('Restoration verification failed');
        rollback.restoredPaths.push(item.path);
      } catch (restoreError) {
        rollback.errors.push({ path: item.path, error: String(restoreError) });
      }
    }
    manifest.rollback = rollback;
    manifest.status = rollback.preservedConcurrentPaths.length || rollback.errors.length ? 'partial-rollback' : 'rolled-back';
    writeFileSync(manifestPath, JSON.stringify(manifest, null, 2) + '\n');
    throw error;
  }
  writeFileSync(manifestPath, JSON.stringify(manifest, null, 2) + '\n');
  return archive;
}
if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const args = process.argv.slice(2);
  if (args.some(x => x !== '--apply')) throw new Error('Usage: node scripts/retire-messaging-data.mjs [--apply]');
  const vault = resolveVaultFromConfig();
  const plan = planRetirement(vault);
  console.log(JSON.stringify({ vault, mode: args.includes('--apply') ? 'apply' : 'dry-run', changes: plan.map(x => ({ path: x.path, reason: x.reason, action: x.after === null ? 'archive' : 'filter' })) }, null, 2));
  if (args.includes('--apply') && plan.length) console.log(JSON.stringify({ recovery: applyRetirement(vault, plan, join(homedir(), 'Library/Application Support/Hollow/retired-messaging')) }));
}
