// Browser-mock Settings, Sources (H4b): `sources_status`, Save of knowledge
// references, and the owner switch's dry run, apply, rollback and status.
// Starts as Alex's setup (tasks in Linear, team ENG; Notion as a
// read-only knowledge reference). `?mockSources=vault` starts vault only.
import type {
  SourceLabel, SourcesBinding, SourcesMigrationPlan, SourcesMigrationRecord, SourcesMigrationTarget, SourcesOwnerKind, SourcesReference,
  SourcesStatus,
} from './generated/ipc';

const get = (key: string) => (typeof location === 'undefined' ? null : new URLSearchParams(location.search).get(key));

let owner: SourcesOwnerKind | undefined;
let references: SourcesReference[] = [{ connector: 'notion-official', label: 'Notion archive' }];
let switches: SourcesMigrationRecord[] = [];

const tasksOwner = () => (owner ??= get('mockSources') === 'vault' ? 'vault' : 'tracker');

function binding(role: SourcesBinding['role'], to: SourcesOwnerKind): SourcesBinding {
  const tracked = to === 'tracker';
  return {
    role, owner: to, inherited: false, chosen: role === 'tasks', workspace: null,
    provider: tracked ? 'linear' : null, providerName: tracked ? 'Linear' : null, teamId: tracked ? 'team-eng' : null, teamKey: tracked ? 'ENG' : null,
    trackerProject: null, wasTracked: null, readOnly: null, reachable: true, unreachableSince: null, problems: [],
  };
}

function label(to: SourcesOwnerKind): SourceLabel {
  const tracked = to === 'tracker';
  return {
    kind: tracked ? 'tracked' : 'stored', text: tracked ? 'Tracked in Linear' : 'Stored in Hollow', providerName: tracked ? 'Linear' : null,
    url: null, openLabel: null, since: null, wasTracked: null, wasTrackedUrl: null,
  };
}

function status(): SourcesStatus {
  const to = tasksOwner();
  const row = (role: SourcesBinding['role'], o: SourcesOwnerKind) => ({
    role, binding: binding(role, o), label: label(o), lastRead: o === 'tracker' ? new Date().toISOString() : null,
    queuedEdits: 0, heldEdits: 0, conflicts: 0, problems: [],
  });
  return {
    rows: [row('tasks', to), row('knowledge', 'vault'), row('personal', 'vault')],
    workspaces: [],
    references: references.map(reference => ({
      reference, name: reference.connector === 'notion-official' ? 'Notion' : reference.connector, inLibrary: true,
      label: { ...label('vault'), kind: 'readOnly', text: `Read-only from ${reference.connector === 'notion-official' ? 'Notion' : reference.connector}` },
    })),
    generation: switches.length, storedInHollow: to === 'vault', unfinishedMigration: null,
    trackerTaskKeys: [], sentTaskKeys: [], trackerProjectKeys: [], sentProjectKeys: [], problems: [],
  };
}

function plan(target: SourcesMigrationTarget): SourcesMigrationPlan {
  const from = tasksOwner();
  return {
    target, from, nothingToDo: target.workspace == null && target.to === from, refused: null,
    files: target.to === 'vault' ? [{ path: 'workspaces/hollow/PROJECT.md', action: 'edit', why: 'Adds "Was tracked in Linear".' }] : [],
    configChange: `config.json: sources.tasks.owner = ${target.to}`, generationFrom: switches.length, generationTo: switches.length + 1,
    storedInHollowAfter: target.to === 'vault', queuedEdits: 0, heldEdits: 0, conflicts: 0, mirroredTasks: 12, mirroredProjects: 3, trackerWrites: 0, notes: [],
  };
}

const COMMANDS = new Set(['sources_status', 'sources_set', 'sources_migration_plan', 'sources_migration_apply', 'sources_migration_rollback', 'sources_migration_status']);

export const isMockSourcesCommand = (command: string) => COMMANDS.has(command);

/** For tests: start again from the knobs. */
export function resetMockSources(): void { owner = undefined; references = [{ connector: 'notion-official', label: 'Notion archive' }]; switches = []; }

export function mockSourcesCommand(command: string, args: Record<string, unknown>): unknown {
  switch (command) {
    case 'sources_status': return status();
    case 'sources_set': {
      const update = (args.update ?? {}) as { tasksOwner?: SourcesOwnerKind | null; references?: SourcesReference[] | null };
      if (update.tasksOwner && update.tasksOwner !== tasksOwner()) throw new Error('Changing where tasks live is a switch, not a Save. Nothing was changed.');
      if (update.references) references = update.references;
      return status();
    }
    case 'sources_migration_plan': return plan(args.target as SourcesMigrationTarget);
    case 'sources_migration_apply': {
      const target = args.target as SourcesMigrationTarget;
      const p = plan(target);
      const record: SourcesMigrationRecord = {
        id: `switch-${switches.length + 1}`, phase: 'applied', target, from: p.from, createdAt: new Date().toISOString(), vault: '/mock/vault',
        generationFrom: p.generationFrom, generationTo: p.generationTo, storedBefore: p.from === 'vault', storedAfter: p.storedInHollowAfter,
        ledgerStep: true, ledgerExisted: true, ledgerBeforeSha: null, ledgerAfterSha: null, configStep: true, configBeforeSha: null, configAfterSha: null,
        rolledBackAt: null, notes: [],
      };
      if (target.workspace == null) owner = target.to;
      switches = [record, ...switches];
      return { plan: p, record, resumed: false, note: '' };
    }
    case 'sources_migration_rollback': {
      const rec = switches.find(s => s.id === args.id);
      if (!rec) throw new Error('No such switch.');
      rec.phase = 'rolledBack'; rec.rolledBackAt = new Date().toISOString();
      if (rec.target.workspace == null) owner = rec.from;
      return rec;
    }
    case 'sources_migration_status': return { switches, unfinished: null };
  }
  return undefined;
}
