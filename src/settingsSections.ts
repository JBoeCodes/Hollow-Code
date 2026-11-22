/**
 * Settings section registry: pure data, no component imports, so the
 * command palette can offer "Settings: X" rows without dragging the
 * (lazy) Settings page chunk into the main bundle.
 *
 * The sections themselves (ids, labels, parts, control labels) and the old
 * ids that still resolve live in `settingsIndex.ts`, which the launch path
 * needs; this module adds every section's, part's and control's search
 * words (`SETTINGS_KEYWORDS`) and owns search. It loads only with the
 * command palette and Settings, both lazy, so the words stay out of the App
 * chunk (build run 11-17, App size pass). Its exports are unchanged.
 *
 * `keywords` feed palette matching; include the names of any old sections
 * a section absorbed so muscle memory still finds them. A new control gets
 * its label in `settingsIndex.ts` and its words here (a vitest guard checks
 * every entry here names a live section, part or control).
 */
import {
  SETTINGS_SECTIONS, type SaveMode, type SettingsControlMeta, type SettingsPartEntry, type SettingsScope, type SettingsSectionEntry,
} from './settingsIndex';

export {
  SETTINGS_ALIAS_CONTROLS, SETTINGS_MOVED_CONTROLS, SETTINGS_SECTION_ALIASES,
  resolveMovedControl, resolveSettingsControl, resolveSettingsSection, settingsSectionForLabel,
} from './settingsIndex';
export type { SaveMode, SettingsControlMeta, SettingsScope } from './settingsIndex';

export interface SettingsSectionMeta extends Omit<SettingsSectionEntry, 'parts'> {
  keywords: string;
  /** Former sections kept whole inside this one (S2), in page order. */
  parts?: readonly SettingsPartMeta[];
}

/** A former section kept whole inside a section (S2). Its id stays its DOM
 *  id (`settings-<id>`) and an alias of the section, so old links land on
 *  it; its label is its heading. */
export interface SettingsPartMeta extends SettingsPartEntry {
  keywords: string;
}

export const SAVE_WORDS: Readonly<Record<SaveMode, string>> = {
  'as-you-change': 'Saved as you change it',
  'save-button': 'Saved when you press its Save',
  'mixed': 'Switches and choices save as you change them; forms keep their own Save',
  'actions': 'Nothing to save: each button acts when you press it',
};

/** The one line a section shows instead of "About these settings" (overnight
 *  11-08 S5, D4). Save mode is per section. A restart sentence only when the
 *  scope says so; none of the twelve sections do. */
export function saveNote(scope: SettingsScope | undefined): string {
  if (!scope) return 'Saves at once.';
  if (/restart/i.test(scope.takesEffect)) return 'Needs a restart.';
  if (scope.saves === 'save-button') return 'Needs Save.';
  if (scope.saves === 'mixed') return 'Saves at once. Forms need Save.';
  return 'Saves at once.';
}

export interface SettingsSectionGroup {
  label: string;
  /** One line on what the group holds (index tooltip). */
  hint?: string;
  items: SettingsSectionMeta[];
}

/** Search words for one section or part: its own, then its controls' by
 *  visible label (a control with none is left out). */
export interface PartWords {
  keywords: string;
  controls?: Readonly<Record<string, string>>;
}
export interface SectionWords extends PartWords {
  parts?: Readonly<Record<string, PartWords>>;
}

/** Every section's search words, by section id, then part id and control
 *  label (from the one table this module held until build run 11-17). */
export const SETTINGS_KEYWORDS: Readonly<Record<string, SectionWords>> = {
  general: {
    keywords: 'general interface appearance theme dark light glass custom palette icons notifications toasts nudges bell startup login tray menu bar window',
    controls: {
      'Appearance': 'theme dark light glass palette colors match macOS system contrast transparency',
      'App icon': 'upload generate logo icon dock image png svg reset preset presets threshold cut glass aurora monolith',
      'COO icon': 'upload generate logo icon orb image png svg reset',
      'Texture': 'full quiet off dither ascii stipple identity',
      'Ambient motion': 'experimental visuals ambient light highlight depth experiments',
      'Motion': 'motion animation atmosphere cinematic reduced reduce motion transitions effects',
      'Density': 'density compact comfortable spacing tight dense rows',
      'Text size': 'text size font size larger smaller zoom scale accessibility',
      'Text font': 'font typeface family interface labels serif system rounded type',
      'Code font': 'font typeface monospace code mono menlo sf mono courier type',
      'Import VS Code theme': 'vscode visual studio code theme json import colours colors custom palette',
      'In-app pop-ups': 'toasts notifications',
      'macOS system notifications': 'notifications banners alerts',
      'Proactive nudges': 'nudges reminders suggestions',
      'Run at login': 'startup launch login item',
      'Keep running when I close the window': 'tray menu bar close hide',
    },
  },
  chat: {
    keywords: 'chat canvases canvas visual answers html charts dashboards replies how often composer send title permission concurrency',
    controls: {
      'Canvases': 'canvas visual charts off when it helps most replies frequency',
      'Enter sends a message': 'enter send key keyboard composer newline shift return',
      'Retitle new chats': 'chat title rename conversation name model retitle',
      'Permission mode for new chats': 'permissions permission mode bypass plan accept edits new chats default revert danger',
      'Chats running at once': 'chats at once concurrency parallel limit simultaneous ten pressure queue',
      'Chats at once in one workspace': 'workspace concurrency parallel limit simultaneous five per workspace',
      'Subagents running at once from one chat': 'subagents per chat helpers children cap parent limit simultaneous six queue waiting coo lead start session',
    },
  },
  models: {
    keywords: 'models agents default agent model provider claude codex antigravity grok effort runtime',
    controls: {
      'Default agent': 'provider runtime',
      'When an agent runs out': 'usage limit out of usage runs out quota wait reset continue fallback another agent switch provider',
      'Models': 'model choice custom not installed',
      'Effort': 'reasoning effort thinking automatic fixed cfo effort per job kind',
      'Models the CFO may use': 'cfo allowlist allowed models subagents coo unattended which model choose',
    },
    parts: {
      selfhosted: {
        keywords: 'self hosted selfhosted own gpu server vllm qwen open weights endpoint forwarder served model context window wake test api style anthropic messages openai responses codex',
        controls: {
          'Add a self-hosted model': 'endpoint server vllm qwen add',
          'Use the Qwen on Cloud Run preset': 'qwen cloud run preset fill',
        },
      },
    },
  },
  accounts: {
    keywords: 'accounts sign in login logout claude chatgpt openai codex antigravity grok xai jev typesafe api key multiple plans max seats switch rotate spread limits five hour weekly extra overage subscriptions update upgrade version cli',
    controls: {
      'Claude': 'anthropic plans',
      'Other providers': 'codex chatgpt grok sign in',
      'Jev (TypeSafe)': 'jev typesafe api key keychain manage key',
      '1Password': 'onepassword 1password op secrets vault reference op:// keychain password manager credentials read secret cli',
      'Agent updates': 'update upgrade version cli install claude code codex antigravity grok npm homebrew automatic latest',
      'Update all': 'update every agent all at once cli versions latest',
    },
  },
  personalization: {
    keywords: 'personalization knowledge profile operator identity communication anti-patterns guardrails decisions me review freshness context global about you memory learnings proposals trusted corrections preferences approve recall remember',
    controls: {
      'Share with agents': 'profile context global',
      'Your profile': 'facts identity',
      'Waiting for you': 'profile suggestions questions suggested facts pending count',
      'Memory': 'memory learnings recall remember trusted approved saved learning memories review outdated superseded replaced old corrections library',
      'How many learnings to suggest': 'learning suggestions fewer less important off all impact proposals chore',
    },
  },
  connectors: {
    keywords: 'connectors connections mcp servers shared library claude codex superhuman mail import plaud notion data sources integration agent tools teamwork computer control task tools hollow_tasks search_vault vault tool calendar eventkit mac agenda permission',
    controls: {
      'Services': 'github setup calendar connection',
      'Agenda calendar': 'calendar mac microsoft off selection permission',
      'Built into Hollow': 'hollow tools tasks app vault search teamwork computer',
      'Your connectors': 'service health sync status mcp servers agent access use hollow sign-in oauth zoom wispr every account codex notion official archive',
      'Add connectors': 'catalog install new',
      'Recordings': 'meetings plaud transcripts calendar',
      'Recordings in Hollow': 'plaud sign in connect disconnect recordings agents connector',
      'Skills are in Library': 'skills capabilities templates instructions agent skills slash commands new skill import skill export available to catalog library',
    },
  },
  sources: {
    keywords: 'sources tasks and projects task tracker linear tracker issues sync interval hollow folder vault files storage',
    controls: {
      'Where your work lives': 'sources of truth roles owner tracked in stored in hollow',
      'Tasks and projects': 'task tracker linear owner where tasks live vault only vault plus linear switch roll back dry run',
      'Knowledge': 'knowledge notes docs meetings references read only connectors notion archive',
      'Personal context': 'personal context profile memory learnings always in your hollow folder',
      'Workspace task sources': 'workspace override inherits tasks source vault only personal workspace',
      'Task tracker': 'linear tracker tasks projects issues sync now interval api key keychain team engineering requests left rate limit last sync next sync errors outbox',
      'Run setup again': 'first run setup onboarding welcome steps import conversations claude code codex scan context agents start again reopen',
    },
    parts: {
      vault: {
        keywords: 'files storage obsidian markdown notes paths vault projects directories folder home source workspace setup location home path inside onboarding start locally',
        controls: {
          'Bring existing folders': 'projects folder existing vault obsidian link elsewhere migrate',
          'Move Hollow home': 'move vault projects into hollow folder gather consolidate icloud documents out of documents personal archive check the move dry run quit and move roll back rollback undo',
        },
      },
      layers: {
        keywords: 'context layers personal context org organization repo repository git sync remote home machine slug project folders claude.md agents.md vault github private create connect local only access check request access',
        controls: {
          'Home': 'machine slug this mac home file personal clone folder',
          'Personal': 'personal context me memory learnings sync now stop syncing connect create repo remote local only last sync',
          'Organization': 'org organization company read only clone access check request access pull',
          'Project folders': 'projects claude.md agents.md repos',
          'Context check': 'leak checks pre-push hook install context check remove secrets flag block audit',
          'Vault sync': 'vault remote push held',
        },
      },
    },
  },
  background: {
    keywords: 'background work automation routines heartbeat beats run queue schedule windows overnight cap',
    controls: {
      'Scheduled automations': 'schedule cron routines',
      'Automations, windows and the run queue are on the Automations page': 'automation windows tonight nightly friday weekday evening hours days schedule window time zone run queue queue waiting cancel add automation',
      'Silent heartbeat': 'heartbeat background check in',
      'COO check-ins': 'coo main agent check in interval proactive',
      'Act on its own': 'coo main agent autonomy suggest act start chats',
      'Start fresh after an hour away': 'coo main agent fresh session resume away large context cost cache',
      'Largest swarm (chats)': 'swarm coo main agent size children parts',
      'Swarm parts at once': 'swarm concurrency parallel simultaneous',
      'Swarm model': 'swarm model haiku sonnet claude',
    },
  },
  devices: {
    keywords: 'devices companion menu bar app iphone ipad watch web pair pairing code revoke hollow tasks sync homelab backend server runner mac mini',
    controls: {
      'Pair a device': 'pairing code connect menu bar',
      'iPhone app': 'iphone phone mobile chats chat api tailscale serve publish tailnet pair an iphone qr code switch server 47680',
      'Backend for the web app': 'homelab backend server url applier token hollow-companiond tailscale connect',
      'What the backend copies': 'scope personal work data workspaces notes mirror homelab',
      'COO on this Mac': 'dispatcher runner route',
      'Move running chats to the runner when this Mac sleeps': 'sleep lid close closed move chats runner mac mini keep going laptop',
      'Remote runner': 'mac mini runner remote ssh off set up owner authorized keys turn off second computer always on',
      'Runs on': 'mac mini runner remote ssh this mac where agent runs build workspace offload machine',
      'Builds and tests': 'mac mini runner hollow-slot route routing auto gate cargo test vitest build where builds run offload this mac remote',
      'Overnight and worker chats': 'mac mini night queue overnight unattended worker batch swarm runner offload fallback when it answers',
      'Runner GitHub access': 'runner github token gh login keychain push pull request',
    },
  },
  usage: {
    keywords: 'tokens cost spend estimated reported api subscription ledger metering codex claude budgets response feedback limits plan five hour weekly all agents grok antigravity self-hosted',
    controls: {
      'Read live from Claude': 'show claude plan usage quota service keychain live',
      'In rotation': 'claude account use this account rotation enabled',
      'Automatic account rotation': 'claude accounts spread rotate plans routing failover fail over account runs out limit switch next account headroom',
      'Move on at': 'ceiling percent spread skip account threshold',
      'Check usage': 'claude accounts refresh five hour weekly read now',
      'Spending caps': 'main agent overnight cap hard stop',
      'By model': 'opus sonnet gpt qwen model',
      'Outcomes': 'finished stopped failed moved another account',
      'Usage by day': 'daily chart agent night queue estimated reported',
      'Budgets': 'limit cap spend',
      'Reserve for your own prompts': 'cfo reserve headroom room left limits unattended subagents accounts 5-hour weekly hold',
      'Extra usage for unattended work': 'cfo extra usage overage paid credits approve withdraw',
      'Budget for each subagent': 'cfo subagent budget cap tokens minutes pause parent',
      'Project budgets': 'cfo project budget lead subagents tokens 5-hour weekly hold',
      'COO daily hard stop': 'coo main agent cap hard stop daily spend dollars',
      'Overnight cap': 'overnight cap night budget hard stop tonight',
      'Value this month': 'value baseline baselines minutes saved hours saved per accepted run',
    },
  },
  shortcuts: {
    keywords: 'keyboard shortcuts hotkeys keys bindings keymap accelerators remap rebind turn off disable command help',
    controls: {
      'Use keyboard shortcuts': 'off disable all turn off on',
      'Change': 'change a shortcut record rebind remap custom key remove',
      'Reset all to defaults': 'restore default',
      'Show all keys': 'help overlay all keys cheat sheet every key card in-surface keys',
      'Swipe to go back and forward': 'trackpad two finger swipe gesture history back forward navigate',
    },
  },
  diffs: {
    keywords: 'diff diffs changes wrap long lines line numbers hunk build workspace git',
    controls: {
      'Wrap long lines': 'wrap word wrap horizontal scroll sideways long lines diff hunk',
      'Show line numbers': 'numbers gutter old new line numbers diff',
    },
  },
  browser: {
    keywords: 'browser links web links open preview pane default browser safari chrome click',
    controls: {
      'Open web links in': 'preview pane default browser external safari chrome links reply command shift click',
    },
  },
  worktrees: {
    keywords: 'worktrees worktree git build workspace branches clean up disk space finished merged',
    controls: {
      'Build workspace worktrees': 'worktree git branch clean up remove finished merged disk space report hygiene',
    },
  },
  archived: {
    keywords: 'archived archive settled conversations chats restore unarchive bring back',
    controls: {
      'Archived conversations': 'archived settled restore unarchive chats bring back old',
    },
  },
  updates: {
    keywords: 'hollow updates version main nightly channel overnight install server credential connection',
    controls: {
      'Channel': 'main nightly release',
      'Install updates overnight': 'automatic installer',
      'Check now': 'refresh updates version',
      'Update token': 'credential connect',
      'Connect Hollow': 'update server',
      'Disconnect': 'update server',
    },
  },
  about: {
    keywords: 'about hollow welcome onboarding first run setup start agent acp registry import',
    controls: { 'Welcome to Hollow': 'open welcome page setup again reopen agents ready details start without agent' },
  },
  advanced: {
    keywords: 'advanced system config agents binaries debug logs diagnostics runtime capability doctor backup health prices developer watchdog deadline run limits app builds',
    controls: {
      'This Mac': 'health doctor backup disk runtime',
      'Capability doctor': 'doctor checks health diagnostics system health',
      'Backup evidence': 'backup time machine',
      'Backup management': 'backup managed elsewhere another backup system time machine',
      'History & recovery is in Library': 'history recovery restore snapshots undo revert runs diff automated writes changed files conversation history library',
      'Runtime': 'configuration binaries agents runtime capabilities which agent supports connectors tools allowlists assistance import',
      'Disk space': 'disk free space storage low full caches worktrees build folders warning threshold warn under vault drive delete old install builds remove finished worktrees clean up cleanup prune old search indexes embeddings',
      'Hollow’s own model calls': 'extraction titles background jobs model calls',
      'Extraction model': 'extraction commitments records decisions people haiku recordings',
      'Background jobs': 'background jobs self hosted private data personal hr payroll claims recording extraction titles learning meeting recap prep pack about you scan heartbeat queue waiting wake run them now',
      'Run limits': 'watchdog deadline timeout stop stalled long run per agent',
      'No-output watchdog': 'timeout stall silent quiet',
      'Run deadline': 'timeout time limit timed out',
      'App builds and web backend': 'build app web backend',
      'App builds': 'build app install hollow compile release bundle sign codesign mac mini runner remote this mac',
      'Prices': 'price per million model rate codex estimate',
      'Settings file': 'export import settings restore json move preferences',
      'Diagnostics': 'developer readouts support startup warm-up',
      'Diagnostics bundle': 'save diagnostics zip logs receipts records redacted support bug report',
      'Build identity': 'version commit about build installed copy',
      'Folders agents can run tools from': 'PATH gcloud command not found shell zshrc tools bin',
      'Launch calls': 'launch ipc calls backend startup performance budget repeated slow',
      "Hollow's databases": 'database sqlite index wal sessions search embeddings integrity quick check size',
      'Debug logs': 'logs debugging',
    },
    parts: {
      indexes: {
        keywords: 'semantic index embeddings search meaning vault keyword content contents pdf office full text local index jev typesafe relevance rerank prioritize context faster efficient cheaper passages connectors duplicates',
        controls: {
          'Search inside project files': 'content index contents pdf office word excel powerpoint full text bm25 local index',
          'Search by meaning': 'semantic meaning embeddings library',
          'Index my vault for semantic search': 'embeddings semantic meaning index',
          'Indexing progress': 'backfill status',
          'Context prioritizer': 'prioritize context local ranking relevance',
          'Jev relevance model': 'typesafe key rank rerank relevance use jev to rank context passages',
          'Jev duplicate check': 'typesafe key duplicates repeats suggestions use jev to check suggestions for duplicates',
        },
      },
    },
  },
};

/** A control with its search words, when it has any. */
function withWords(c: SettingsControlMeta, words: Readonly<Record<string, string>> | undefined): SettingsControlMeta {
  const keywords = words?.[c.label];
  return keywords === undefined ? c : { ...c, keywords };
}

/** A section's entry with its, its parts' and its controls' search words. */
function withKeywords(s: SettingsSectionEntry): SettingsSectionMeta {
  const w = SETTINGS_KEYWORDS[s.id];
  const partWords = (id: string | undefined) => (id ? w?.parts?.[id] : undefined);
  const { parts: entryParts, ...rest } = s;
  const parts = (entryParts ?? []).map(p => ({ ...p, keywords: partWords(p.id)?.keywords ?? '', controls: p.controls.map(c => withWords(c, partWords(p.id)?.controls)) }));
  return {
    ...rest,
    // The parts' names find the section too (palette rows, the index search).
    keywords: [w?.keywords ?? '', ...parts.map(p => p.label.toLowerCase())].join(' '),
    ...(s.controls ? {
      controls: s.controls.map(c => (c.heading ? { ...c, keywords: partWords(c.part)?.keywords ?? '' } : withWords(c, c.part ? partWords(c.part)?.controls : w?.controls))),
    } : {}),
    ...(entryParts ? { parts } : {}),
  };
}

export const SETTINGS_SECTION_GROUPS: readonly SettingsSectionGroup[] = [
  { label: 'Settings', items: SETTINGS_SECTIONS.map(withKeywords) },
];

/** Every live section id, flat, in index order. */
export const SETTINGS_SECTION_IDS: readonly string[] =
  SETTINGS_SECTION_GROUPS.flatMap(g => g.items.map(s => s.id));

/** A section a settings search found, with the controls that matched. */
export interface SettingsSearchHit { group: string; section: SettingsSectionMeta; controls: SettingsControlMeta[] }

const words = (s: string) => s.toLowerCase().normalize('NFKD').replace(/\p{M}/gu, '');

/**
 * Settings search (audit H06): every typed term must appear in the section's
 * own text, or in one control's label/keywords (the section's name, and its
 * part's, count as context there). Sections come back in index order, each
 * with the controls that matched so the page can jump to them. A query that
 * only names a section or part lists the controls it names, not all of them
 * (a part's heading is one, so "accounts" finds Accounts). An empty
 * query returns all.
 */
export function searchSettings(query: string, groups: readonly SettingsSectionGroup[] = SETTINGS_SECTION_GROUPS): SettingsSearchHit[] {
  const terms = words(query).split(/\s+/).filter(Boolean);
  const names = (text: string) => terms.every(t => words(text).includes(t));
  const out: SettingsSearchHit[] = [];
  for (const g of groups) for (const section of g.items) {
    if (!terms.length) { out.push({ group: g.label, section, controls: [] }); continue; }
    const partOf = (c: SettingsControlMeta) => (c.part && section.parts?.find(p => p.id === c.part)?.label) || '';
    const controls = (section.controls ?? []).filter(c => names(`${section.label} ${partOf(c)} ${c.label} ${c.keywords ?? ''}`))
      .filter(c => !(names(section.label) || (partOf(c) && names(partOf(c)))) || names(c.label));
    if (controls.length || names(`${section.label} ${section.id} ${section.keywords}`)) out.push({ group: g.label, section, controls });
  }
  return out;
}

/** The control a narrow window's section picker lands on (overnight 11-13
 *  S1): the words typed in its search found the section by a control, so
 *  the jump goes to the control they name (else the first they match). None for no words, or for
 *  words that name the section itself ("usage" picks Usage, not Check usage). */
export function pickerControl(section: string, query?: string): string | undefined {
  const terms = words(query ?? '').split(/\s+/).filter(Boolean);
  const meta = SETTINGS_SECTION_BY_ID[section];
  if (!terms.length || !meta || terms.every(t => words(`${meta.label} ${meta.id}`).includes(t))) return undefined;
  const found = searchSettings(query ?? '').find(h => h.section.id === section)?.controls ?? [];
  // One its own label names before a part heading its keywords matched.
  return (found.find(c => terms.every(t => words(c.label).includes(t))) ?? found[0])?.label;
}

/** The part a control sits in, when it is a part's heading (S2): a jump to
 *  it lands on the part itself, never on a row whose name starts the same. */
export function settingsPartHeading(section: string, label?: string): string | undefined {
  const c = label ? SETTINGS_SECTION_BY_ID[section]?.controls?.find(x => x.label === label) : undefined;
  return c?.heading ? c.part : undefined;
}

/** Each section's registry row by id. */
export const SETTINGS_SECTION_BY_ID: Readonly<Record<string, SettingsSectionMeta>> = Object.fromEntries(
  SETTINGS_SECTION_GROUPS.flatMap(g => g.items).map(s => [s.id, s]));
