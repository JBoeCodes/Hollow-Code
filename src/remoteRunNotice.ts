// The Mac mini runner (2026-11-01, Phase 2): the chat's quiet system lines
// for a turn whose agent ran on the remote runner (`remoteRun` events from
// chat.rs and remote_session.rs). Where it ran, then how its work came back.
// Phase 3 (2026-11-02): a vault workspace's folder, scope `folder`.
// Overnight P3L-4: an earlier result on a base this Mac never sent comes
// back only as copies beside the files (`unsure`).
// Overnight R3: what a workspace folder's copy went without (`leftHere`),
// and what of its return didn't come (`notBrought`, each kind the first few
// and a count, so thousands stay one line).
// Self-hosted models Q7: a self-hosted turn whose Runs on is the runner runs
// here, and its `stayed` line (with `selfHosted`) says why, and whether work
// an earlier turn left on the runner is still there.
// RT1 (overnight 11-17): `start` and `stayed` carry `route` (where the turn
// ran, why, and whether that was a fallback), and the line says why.

/** One kind of path: how many, and the first few. */
type LeftKind = { count?: number; first?: string[] };
type LeftHere = { secret?: LeftKind | null; big?: LeftKind | null; repos?: LeftKind | null; budget?: LeftKind | null };
type NotBrought = { skipped?: LeftKind | null; refused?: LeftKind | null; tooBig?: LeftKind | null; pastBudget?: LeftKind | null };
/** `skipped` as a plain list is what turns recorded before R3's review say. */
type FolderEarlier = { changed?: number; merged?: string[]; copies?: string[]; unsure?: string[]; keptHere?: string[]; skipped?: string[]; failed?: string[]; notBrought?: NotBrought | null };

/** Remote runners (2026-11-03): GitHub's source (or why there is none) and
 *  Google Cloud's access (or `no-gcloud`), from the start event. */
type RunnerReach = { github?: 'runner-token' | 'gh-login' | 'no-token' | 'no-gh-login' | null; repo?: string | null; cloud?: string | null; serviceAccount?: string | null };

/** The start line's sentence for `reach`, or '' when the turn reaches nothing more. */
export function reachText(reach: RunnerReach | null | undefined): string {
  if (!reach) return '';
  const parts: string[] = [];
  const repo = reach.repo ? ` (${reach.repo})` : '';
  switch (reach.github) {
    case 'runner-token': parts.push(`GitHub${repo} with your runner token`); break;
    case 'gh-login': parts.push(`GitHub${repo} with this Mac's gh login`); break;
    case 'no-token': parts.push('no GitHub: no runner token is saved (Settings → Devices → Runner GitHub access)'); break;
    case 'no-gh-login': parts.push("no GitHub: this Mac's gh isn't signed in"); break;
  }
  switch (reach.cloud) {
    case 'limited': parts.push('Google Cloud as you, no production changes'); break;
    case 'full': parts.push('Google Cloud as you'); break;
    case 'service-account': parts.push(`Google Cloud as ${reach.serviceAccount ?? 'a service account'}`); break;
    case 'no-gcloud': parts.push("no Google Cloud: gcloud isn't installed on this Mac"); break;
  }
  return parts.length ? ` This turn reaches ${parts.join('; ')}.` : '';
}

/** Self-hosted models Q7: the model a turn kept on this Mac runs on, how
 *  Hollow signs in to its endpoint (`self_hosted::sign_in_kind`), and (Q7
 *  review) what of an earlier turn is still on the runner: `pending`, a turn
 *  stopped by a quit that the runner didn't answer for, or wasn't asked
 *  about because it didn't answer minutes ago (a later message tries
 *  again), or `runner`, work only a turn there brings back. */
type SelfHostedStay = { model?: string; signIn?: 'gcloud' | 'command' | 'none' | null; leftOnRunner?: 'pending' | 'runner' | null };

/** RT1: a routing decision as recorded (`remote_session::Route`). `cause`
 *  is the machine-readable why; `reason` is its plain words, used when this
 *  build doesn't know the cause. */
export type RunRoute = {
  target: 'runner' | 'here';
  host: string;
  cause?: 'chosen' | 'preferred' | 'reattached' | 'unreachable' | 'refusing' | 'busy' | 'signIn' | 'prepare' | string;
  reason?: string;
  fallback?: boolean;
};

/** RT1: why a turn ran where it did, as the end of "... because <this>". */
export function routeBecause(route: RunRoute | null | undefined): string {
  if (!route) return '';
  const host = route.host?.trim() ? ` (${route.host.trim()})` : '';
  switch (route.cause) {
    case 'unreachable': return `the runner${host} did not answer`;
    case 'refusing': return `the runner${host} wasn't taking work`;
    case 'busy': return `another turn of this worktree was still on the runner${host}`;
    case 'signIn': return "this turn's sign-in stays on this Mac";
    case 'prepare': return `the runner${host} couldn't be made ready`;
    case 'chosen': return 'Runs on is set to the runner';
    case 'preferred': return 'overnight and worker chats go to the runner when it answers';
    case 'reattached': return 'it picks up the run an earlier Hollow left there';
    default: return route.reason?.trim().replace(/[.]$/, '') ?? '';
  }
}

/** Q7: why a self-hosted turn whose Runs on is the runner runs here. Q7
 *  review: the reason that always holds (the runner can't reach this Mac's
 *  local forwarder, and the sign-in is here), never "no Google Cloud login",
 *  which a Google Cloud runner's own service account could make false. */
function selfHostedStayText(host: string, stay: SelfHostedStay): string {
  const model = stay.model?.trim() || 'this self-hosted model';
  const left = stay.leftOnRunner === 'pending'
    ? " The turn that was running on the runner when Hollow quit wasn't brought back: the runner didn't answer (or wasn't asked, because it didn't answer minutes ago), so its work is still there and it may still be running; a later message in this chat brings it back once the runner answers."
    : stay.leftOnRunner === 'runner'
      ? " Work an earlier turn left on the runner stays there until a turn there on a model that isn't self-hosted brings it back."
      : '';
  // Q7 review: Runs on is the runner but none is set up (an empty host).
  if (!host.trim()) return `Runs on is the remote runner, but none is set up on this Mac; ${model} runs on this Mac either way, so this turn runs here.${left}`;
  const signIn = stay.signIn === 'gcloud'
    ? " and signs in with this Mac's gcloud"
    : stay.signIn === 'command'
      ? ' and signs in with its token command on this Mac'
      : '';
  return `The runner (${host.trim()}) can't use ${model}: Hollow serves it through this Mac's local forwarder${signIn}, so this turn runs on this Mac.${left}`;
}

export type RemoteRunEvent =
  | {
      type: 'remoteRun';
      phase: 'start';
      host: string;
      dir?: string;
      kept?: string[] | string | null;
      scope?: 'worktree' | 'folder';
      earlier?: FolderEarlier | null;
      /** Idle sessions' folders the runner cleared for disk space (overnight N5). */
      pruned?: { count?: number; freeGb?: number; floorGb?: number } | null;
      /** Copies of a run that died there, kept an hour without luck, moved aside on the Mac mini (overnight P3L-5). */
      aside?: string[] | null;
      /** 11-02: this Mac has no git identity for the worktree, so commits there carry the Mac mini's default. */
      noIdentity?: boolean | null;
      /** Overnight R1: this turn picks up a run an earlier Hollow left going there. */
      reattached?: boolean | null;
      /** Remote runners (2026-11-03): what the turn may reach besides its checkout. */
      access?: RunnerReach | null;

      /** Overnight R3: what of the workspace folder stayed on this Mac (secret-looking, over 50 MB, a repository of its own). */
      leftHere?: LeftHere | null;
      /** RT1: why the turn went to the runner. */
      route?: RunRoute | null;
      sessionKey?: string | null;
    }
  | {
      /** Overnight N6: unattended work the Mac mini couldn't take ran here. */
      type: 'remoteRun';
      phase: 'stayed';
      host: string;
      reason?: string;
      /** Self-hosted models Q7: a turn on this model ran here whatever Runs on says. */
      selfHosted?: SelfHostedStay | null;
      /** RT1: where it ran instead, why, and that it was a fallback. */
      route?: RunRoute | null;
      sessionKey?: string | null;
    }
  | {
      type: 'remoteRun';
      phase: 'back';
      host: string;
      outcome: 'applied' | 'unchanged' | 'keptAside' | 'failed';
      scope?: 'worktree' | 'folder';
      branch?: string;
      head?: string;
      message?: string;
      changed?: number;
      merged?: string[];
      copies?: string[];
      keptHere?: string[];
      skipped?: string[];
      notBrought?: NotBrought | null;
      failed?: string[];
      /** Overnight R1: an earlier Hollow's turn, brought back before this one ran. */
      earlier?: boolean | null;
      sessionKey?: string | null;
    };

const files = (n: number) => `${n} file${n === 1 ? '' : 's'}`;

/** A kind's paths: the first few, then how many more. */
function named(k: LeftKind | null | undefined): { count: number; text: string } {
  const first = k?.first ?? [];
  const count = Math.max(k?.count ?? 0, first.length);
  const more = count - first.length;
  return { count, text: `${first.join(', ')}${more > 0 ? `${first.length ? ' and ' : ''}${more} more` : ''}` };
}

/** What a folder's return did, as clauses: written, merged, kept twice. */
function folderParts(changed: number, merged: string[], copies: string[], keptHere: string[] = [], skipped: string[] = [], failed: string[] = [], unsure: string[] = [], notBrought?: NotBrought | null): string[] {
  const parts: string[] = [];
  if (changed) parts.push(files(changed));
  if (merged.length) parts.push(`merged with changes made here: ${merged.join(', ')}`);
  if (copies.length) parts.push(`changed on both sides, so both kept (the runner's as ${copies.join(', ')})`);
  if (unsure.length) {
    const listed = unsure.length > 10 ? `${unsure.slice(0, 10).join(', ')} and ${unsure.length - 10} more` : unsure.join(', ');
    parts.push(`made on a copy this Mac can't confirm it sent, so only kept beside your files, nothing here written over: ${listed}`);
  }
  if (keptHere.length) parts.push(`kept as changed here though removed there: ${keptHere.join(', ')}`);
  const not = notBrought ?? { skipped: { count: skipped.length, first: skipped.slice(0, 5) } };
  const kinds: [LeftKind | null | undefined, string][] = [
    [not.skipped, 'not brought (a link, an unsafe path, or junk or a cache this Mac keeps out)'],
    [not.refused, 'never brought back (key, credential and tool settings files, such as .env)'],
    [not.tooBig, 'left on the runner (over 50 MB)'],
    [not.pastBudget, "left on the runner (past the 2,000 files or 200 MB a turn brings of files only a .gitignore names)"],
  ];
  for (const [kind, why] of kinds) {
    const { count, text } = named(kind);
    if (count) parts.push(`${why}: ${text}`);
  }
  if (failed.length) parts.push(`couldn't be written here: ${failed.join(', ')}`);
  return parts;
}

/** What of a workspace folder stayed on this Mac, as one sentence (or nothing). */
function leftHereText(left: LeftHere | null | undefined, from: string): string {
  const kind = (k: LeftKind | null | undefined, what: (n: number) => string): string => {
    const { count, text } = named(k);
    if (!count) return '';
    return text ? `${what(count)} (${text})` : what(count);
  };
  const parts = [
    kind(left?.secret, (n) => `${n} secret-looking ${n === 1 ? 'item' : 'items'}`),
    kind(left?.big, (n) => `${n} ${n === 1 ? 'file' : 'files'} over 50 MB`),
    kind(left?.repos, (n) => (n === 1 ? '1 repository of its own' : `${n} repositories of their own`)),
    kind(left?.budget, (n) => `${n} ${n === 1 ? 'file' : 'files'} only a .gitignore names, past the 2,000 files or 200 MB a turn takes`),
  ].filter(Boolean);
  if (!parts.length) return '';
  const list = parts.length > 1 ? `${parts.slice(0, -1).join(', ')} and ${parts[parts.length - 1]}` : parts[0];
  return ` Left on this Mac${from}, so the agent there doesn't see them: ${list}.`;
}

/** RT1: the opening clause each cause's own sentence starts with
 *  (`remote_session::unreachable_message`, `refusing_message`,
 *  `remote_status::stalled_message` and the status's "When checked N ago"),
 *  dropped from the detail so the line doesn't say it twice. */
const CAUSE_LEADS: Record<string, RegExp> = {
  unreachable: /^(?:When checked ([^,]+), )?(?:The runner \([^)]*\)|\S+) didn't (?:answer|finish the check in \d+ s)(:\s*|\.\s*|\s+|$)/,
  refusing: /^(?:When checked ([^,]+), )?(?:it|\S+) (?:isn't|wasn't) taking work(?: now)?(\s+|$)/,
};

/** RT1: "Running on this Mac because <why>", then what the reason adds. */
function stayedBecause(because: string, cause: string | undefined, host: string, reason: string, end: (t: string) => string): string {
  // The blocker's own sentence already is the reason: say it once.
  if (reason && (cause === 'busy' || cause === 'signIn')) return `Running on this Mac instead of on the runner (${host}): ${end(reason)}`;
  const lead = cause ? CAUSE_LEADS[cause]?.exec(reason) : null;
  if (!lead) return `Running on this Mac because ${because}.${reason ? ` ${end(reason)}` : ''}`;
  const when = lead[1] ? ` when checked ${lead[1]}` : '';
  const rest = reason.slice(lead[0].length).trim();
  if (!rest) return `Running on this Mac because ${because}${when}.`;
  const sep = lead[2].startsWith(':') ? ': ' : rest.startsWith('(') ? ' ' : '. ';
  return `Running on this Mac because ${because}${when}${sep}${end(rest)}`;
}

/** The line the chat shows for one `remoteRun` event. */
export function remoteRunNotice(event: RemoteRunEvent): string {
  // Overnight R1: after a quit, Continue picks the run up there; any other
  // message first brings that turn's work back.
  if (event.phase === 'start' && event.reattached) {
    const place = event.scope === 'folder' ? "this workspace's folder" : 'this worktree';
    return `Picked up the turn that was still running on the runner (${event.host}) when Hollow quit: the rest of its output follows, and its work comes back to ${place} when it ends. Tools, History and Stop stay on this Mac.`;
  }
  if (event.phase === 'back' && event.earlier) {
    return `The turn that was running on the runner (${event.host}) when Hollow quit was stopped there before this one. ${noticeText(event)}`;
  }
  return noticeText(event);
}

function noticeText(event: RemoteRunEvent): string {
  if (event.phase === 'stayed') {
    if (event.selfHosted) return selfHostedStayText(event.host ?? '', event.selfHosted);
    const reason = event.reason?.trim() ?? '';
    const end = (t: string) => `${t}${/[.!?)]$/.test(t) ? '' : '.'}`;
    // RT1: a recorded route says why in one clause; older records keep their line.
    const because = routeBecause(event.route);
    const lead = because
      ? stayedBecause(because, event.route?.cause, event.host, reason, end)
      : `Running on this Mac: the runner (${event.host}) couldn't take this turn.${reason ? ` ${end(reason)}` : ''}`;
    return `${lead} Overnight and worker chats go there when it can (Settings → Devices → Runs on).`;
  }
  if (event.phase === 'start') {
    const branches = Array.isArray(event.kept) ? event.kept : event.kept ? [event.kept] : [];
    const kept = branches.length > 1
      ? ` Earlier turns' work that never came back is on branches ${branches.join(', ')}.`
      : branches.length === 1
        ? ` An earlier turn's work that never came back is on branch ${branches[0]}.`
        : '';
    const earlier = event.earlier
      ? folderParts(event.earlier.changed ?? 0, event.earlier.merged ?? [], event.earlier.copies ?? [], event.earlier.keptHere ?? [], event.earlier.skipped ?? [], event.earlier.failed ?? [], event.earlier.unsure ?? [], event.earlier.notBrought)
      : [];
    const back = earlier.length ? ` An earlier turn's changes that never came back are here now (${earlier.join('; ')}).` : '';
    const n = event.pruned?.count ?? 0;
    const pruned = n > 0
      ? ` The runner had ${event.pruned?.freeGb ?? 0} GB free, under its ${event.pruned?.floorGb ?? 0} GB floor, so it removed ${n === 1 ? '1 folder' : `${n} folders`} other turns left there, unused for 12 hours or more.`
      : '';
    const asideList = event.aside ?? [];
    const aside = asideList.length
      ? ` What a turn that stopped there left couldn't be brought back for an hour, so the runner kept it aside at ${asideList.join(', ')} (for up to 14 days) and started fresh.`
      : '';
    const reach = reachText(event.access);
    const went = routeBecause(event.route);
    const why = went ? ` It went there because ${went}.` : '';
    if (event.scope === 'folder') {
      const left = leftHereText(event.leftHere, '');
      return `Running on the runner (${event.host}) in its own copy of this workspace's folder. Tools, History and Stop stay on this Mac.${reach}${left}${back}${aside}${pruned}${why}`;
    }
    const identity = event.noIdentity
      ? " This Mac has no complete git name and email for this worktree, so commits made there carry the runner's default (set user.name and user.email here to change that)."
      : '';
    // A build workspace's turn takes its vault folder beside the worktree.
    const left = leftHereText(event.leftHere, " from this workspace's vault folder");
    return `Running on the runner (${event.host}) in its own copy of this worktree. Tools, History and Stop stay on this Mac.${reach}${kept}${left}${back}${aside}${pruned}${identity}${why}`;
  }
  if (event.scope === 'folder') {
    switch (event.outcome) {
      case 'applied': {
        const parts = folderParts(event.changed ?? 0, event.merged ?? [], event.copies ?? [], event.keptHere ?? [], event.skipped ?? [], event.failed ?? [], [], event.notBrought);
        return `The runner's changes to this workspace's folder are here${parts.length ? ` (${parts.join('; ')})` : ''}.`;
      }
      case 'unchanged':
        return "The turn on the runner changed nothing in this workspace's folder.";
      default:
        return event.message ?? "The workspace folder's changes couldn't be brought back from the runner.";
    }
  }
  switch (event.outcome) {
    case 'applied':
      return `The runner's work is back in this worktree${event.head ? ` (HEAD ${event.head.slice(0, 8)})` : ''}.`;
    case 'unchanged':
      return 'The turn on the runner changed no files.';
    case 'keptAside':
      return event.message ?? `This worktree changed meanwhile, so the runner's work is on branch ${event.branch ?? '(unknown)'}.`;
    case 'failed':
      return event.message ?? "The runner's work couldn't be brought back.";
  }
}
