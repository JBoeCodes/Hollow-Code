import { describe, expect, it } from 'vitest';
import {
  LEDGER_OPEN_MAX, describeRow, describeTool, fileKind, groupFiles, ledgerCounts, ledgerDefaultOpen, ledgerGlyphs,
  ledgerSummary, readLineCount,
  type SummaryTool,
} from './toolDescribe';
import { editStats } from './toolDiff';

const call = (name: string, input: unknown = {}, status: SummaryTool['status'] = 'done'): SummaryTool => ({ name, input, status });

describe('describeTool', () => {
  it('leads a command with the reason the agent gave, the command beside it', () => {
    expect(describeTool('Bash', { command: 'git status --porcelain', description: 'Show working tree status' }))
      .toMatchObject({ icon: 'terminal', verb: 'Show working tree status', target: 'git status --porcelain', mono: true });
    // No reason given: the old "Run <command>" row.
    expect(describeTool('Bash', { command: 'ls' })).toMatchObject({ verb: 'Run', target: 'ls' });
    expect(describeTool('Bash', { command: 'npm test', run_in_background: true }).extra).toBe('background');
  });

  it('clips a long command to one line', () => {
    const m = describeTool('Bash', { command: `python3 - <<'EOF'\n${'x = 1\n'.repeat(40)}EOF`, description: 'Update the brief' });
    expect(m.target).not.toContain('\n');
    expect(m.target.length).toBeLessThanOrEqual(80);
    expect(m.target.endsWith('…')).toBe(true);
  });

  it('names the line range of a partial read', () => {
    expect(describeTool('Read', { file_path: '/a/b/src/store/runtime.ts', offset: 1216, limit: 20 }))
      .toMatchObject({ verb: 'Read', target: '…/store/runtime.ts', extra: 'lines 1216–1235' });
    expect(describeTool('Read', { file_path: 'x.md' }).extra).toBe('');
  });

  it('shows what an MCP call acted on, not only its server', () => {
    expect(describeTool('mcp__hollow_app__read_session', { key: 's-20260927-1cdaa6a0', lastTurns: 2 }))
      .toMatchObject({ verb: 'Read a conversation', target: 's-20260927-1cdaa6a0', extra: 'via hollow app' });
    // Searched or read by turn number (repo features R4).
    expect(describeTool('mcp__hollow_app__read_session', { key: 's-1', query: ' invoice total ', fromTurn: 2 }))
      .toMatchObject({ verb: 'Read a conversation', target: 's-1', extra: 'find “invoice total”' });
    expect(describeTool('mcp__hollow_app__read_session', { key: 's-1', query: 'x'.repeat(60) }).extra)
      .toBe(`find “${'x'.repeat(39)}…”`);
    // Cut by code points: an emoji at the cut stays whole (R4 review).
    expect(describeTool('mcp__hollow_app__read_session', { key: 's-1', query: `${'x'.repeat(38)}😀😀😀` }).extra)
      .toBe(`find “${'x'.repeat(38)}😀…”`);
    expect(describeTool('mcp__hollow_vault__read_source', { path: 't.log', query: `${'x'.repeat(38)}😀😀😀` }).extra)
      .toBe(`find “${'x'.repeat(38)}😀…”`);
    expect(describeTool('mcp__hollow_app__read_session', { key: 's-1', fromTurn: 2, toTurn: 5 }).extra).toBe('turns 2–5');
    expect(describeTool('mcp__hollow_app__read_session', { key: 's-1', fromTurn: 7 }).extra).toBe('turn 7');
    expect(describeTool('mcp__hollow_app__run_action', { id: 'history.get_history_run', args: {} }).target)
      .toBe('history.get_history_run');
    // A subagent started or messaged says so plainly (COO rebuild L3).
    expect(describeTool('mcp__hollow_app__start_session', { workspace: 'w', prompt: 'Review the pricing page' }))
      .toMatchObject({ icon: 'bot', verb: 'Started a subagent', target: 'Review the pricing page' });
    expect(describeTool('mcp__hollow_app__send_to_session', { session: 's-1', prompt: 'Also the footer' }))
      .toMatchObject({ verb: 'Messaged a subagent', target: 'Also the footer' });
    // Nothing to name: the server stands in, as before.
    expect(describeTool('mcp__hollow_app__list_workspaces', {}))
      .toMatchObject({ verb: 'List workspaces', target: 'via hollow app' });
    // Hollow's own verbs are unchanged.
    expect(describeTool('mcp__hollow_vault__search_vault', { query: 'pong' }))
      .toMatchObject({ verb: 'Search your vault', target: 'pong' });
    // The on-demand reader (agent effectiveness F3) reads as an action too:
    // the file it read and the lines it asked for.
    expect(describeTool('mcp__hollow_vault__read_source', { path: 'workspaces/w/sources/brief.md', from_line: 120, to_line: 160 }))
      .toMatchObject({ icon: 'book', verb: 'Read a source', target: '…/sources/brief.md', extra: 'lines 120–160' });
    expect(describeTool('mcp__hollow_vault__read_source', { path: 'notes/a.md', from_line: 41 }))
      .toMatchObject({ verb: 'Read a source', target: 'notes/a.md', extra: 'from line 41' });
    expect(describeTool('mcp__hollow_vault__read_source', { path: 'notes/a.md' }))
      .toMatchObject({ verb: 'Read a source', target: 'notes/a.md', extra: '' });
    // Asked by intent (repo features R3): what it looked for, cut at 40.
    expect(describeTool('mcp__hollow_vault__read_source', { path: '/private/tmp/hollow-run/a/t.log', query: 'E0308', from_line: 3 }))
      .toMatchObject({ verb: 'Read a source', extra: 'find “E0308”' });
    expect(describeTool('mcp__hollow_vault__read_source', { path: 't.log', query: 'x'.repeat(60) }).extra)
      .toBe(`find “${'x'.repeat(39)}…”`);
  });

  it('describes delegation, skills and tool loading', () => {
    expect(describeTool('Agent', { description: 'Find auth code', subagent_type: 'Explore' }))
      .toMatchObject({ verb: 'Delegate', target: 'Find auth code', extra: 'Explore' });
    expect(describeTool('Task', { subagent_type: 'Plan' })).toMatchObject({ verb: 'Delegate', target: 'Plan', extra: '' });
    expect(describeTool('Skill', { skill: 'docx' })).toMatchObject({ verb: 'Use skill', target: 'docx' });
    expect(describeTool('ToolSearch', { query: 'select:Read,Edit' })).toMatchObject({ verb: 'Load tools', target: 'Read,Edit' });
  });
});

describe('ledgerSummary', () => {
  it('says what a single call did', () => {
    const s = ledgerSummary([call('Bash', { command: 'ls -la', description: 'List workspace files' })]);
    expect(s.text).toBe('List workspace files');
    expect(s.icon).toBe('terminal');
    expect(s.failed).toBe(0);
    expect(s.running).toBeUndefined();
  });

  it('names single calls and counts repeated ones, in the order they first ran', () => {
    const s = ledgerSummary([
      call('Bash', { command: 'a' }),
      call('Read', { file_path: '/repo/src/components/ToolLedger.tsx' }),
      call('Bash', { command: 'b' }),
      call('Edit', { file_path: '/repo/PLAN.md' }),
      call('Edit', { file_path: '/repo/WORKSPACE.md' }),
    ]);
    expect(s.text).toBe('Ran 2 commands, read ToolLedger.tsx, edited 2 files');
    // The family with the most calls picks the icon (a tie keeps the first).
    expect(s.icon).toBe('terminal');
  });

  it('says one file was edited several times', () => {
    const s = ledgerSummary([call('Edit', { file_path: '/r/a.ts' }), call('Edit', { file_path: '/r/a.ts' })]);
    expect(s.text).toBe('Edited a.ts 2 times');
    expect(s.icon).toBe('filePen');
  });

  it('keeps three families and counts the rest', () => {
    const s = ledgerSummary([
      call('Read', { file_path: 'a' }), call('Grep', { pattern: 'x' }), call('Bash', { command: 'c' }),
      call('WebSearch', { query: 'q' }), call('TodoWrite', { todos: [] }),
    ]);
    expect(s.text).toBe('Read a, search x, run c +2 more');
  });

  it('collapses repeated MCP calls of one kind', () => {
    const s = ledgerSummary([
      call('mcp__hollow_app__read_session', { key: 'a' }),
      call('mcp__hollow_app__read_session', { key: 'b' }),
    ]);
    expect(s.text).toBe('Read a conversation ×2');
    const mixed = ledgerSummary([
      call('mcp__hollow_app__read_session', { key: 'a' }),
      call('mcp__hollow_app__list_sessions', {}),
    ]);
    expect(mixed.text).toBe('2 hollow app calls');
  });

  it('reports the running call and the failures', () => {
    const s = ledgerSummary([
      call('Read', { file_path: 'a' }, 'error'),
      call('Bash', { command: 'npm test', description: 'Run the unit tests' }, 'running'),
    ]);
    expect(s.running).toMatchObject({ verb: 'Run the unit tests', target: 'npm test' });
    expect(s.failed).toBe(1);
  });

  it('is empty for no calls', () => {
    expect(ledgerSummary([])).toMatchObject({ text: '', failed: 0 });
  });
});

describe('row widgets', () => {
  it('picks a file chip icon by file type', () => {
    expect(fileKind('/a/image.png')).toEqual({ icon: 'fileImage', tone: 'image' });
    expect(fileKind('CLAUDE.md')).toEqual({ icon: 'markdown', tone: 'doc' });
    expect(fileKind('src/ToolCall.tsx')).toEqual({ icon: 'fileCode', tone: 'code' });
    expect(fileKind('notes.txt')).toEqual({ icon: 'fileText', tone: 'plain' });
  });

  it('counts only the numbered lines of a Read result', () => {
    expect(readLineCount('     1→# Title\n     2→\n     3→Body\n\n<system-reminder>x</system-reminder>')).toBe(3);
    expect(readLineCount('10\tfoo\n11\tbar')).toBe(2);
    expect(readLineCount(undefined)).toBe(0);
  });

  it('says how much a read returned once its result is in', () => {
    const r = describeRow('Read', { file_path: '/w/CLAUDE.md' }, '     1→a\n     2→b\n     3→c');
    expect(r).toMatchObject({ verb: 'Read 3 lines', file: '/w/CLAUDE.md', extra: '' });
    // Still running: no count yet.
    expect(describeRow('Read', { file_path: '/w/CLAUDE.md' }).verb).toBe('Read');
    expect(describeRow('Read', { file_path: '/w/x.ts', offset: 40, limit: 2 }, '    40→a\n    41→b').extra).toBe('from line 40');
    expect(describeRow('Read', { file_path: '/w/image.png' }, '').verb).toBe('Read image');
  });

  it('counts a write\'s lines and an edit\'s additions and deletions', () => {
    expect(describeRow('Write', { file_path: 'a.md', content: 'one\ntwo\n' }).verb).toBe('Write 2 lines');
    expect(describeRow('Edit', { file_path: 'a.md', old_string: 'x\ny', new_string: 'x\nz\nw' }).stats)
      .toEqual({ additions: 2, deletions: 1 });
    expect(editStats('MultiEdit', { edits: [{ old_string: 'a', new_string: 'b' }, { old_string: '', new_string: 'c' }] }))
      .toEqual({ additions: 2, deletions: 1 });
    expect(editStats('Bash', { command: 'ls' })).toBeNull();
  });

  it('gives each kind of call its own icon', () => {
    expect(describeTool('Agent', { description: 'x' }).icon).toBe('bot');
    expect(describeTool('Edit', { file_path: 'a' }).icon).toBe('filePen');
    expect(describeTool('ToolSearch', { query: 'select:WebFetch' }).icon).toBe('wrench');
    expect(describeTool('mcp__hollow_tasks__list_tasks', {}).icon).toBe('checkSquare');
    expect(describeTool('mcp__blender__get_scene_info', {}).icon).toBe('plug');
    expect(describeTool('TodoWrite', { todos: [{ status: 'completed' }, { status: 'pending' }] }).target).toBe('1/2 done');
  });
});

describe('ledger header widgets', () => {
  it('counts tool calls and subagents separately', () => {
    expect(ledgerCounts([call('Read'), call('Bash'), call('Agent')])).toBe('2 tool calls, 1 subagent');
    expect(ledgerCounts([call('Read')])).toBe('1 tool call');
    expect(ledgerCounts([call('Task'), call('Agent')])).toBe('2 subagents');
  });

  it('shows one icon per call, capped', () => {
    const tools = [call('Read', { file_path: 'a.md' }), call('Bash', { command: 'ls' }, 'error'), call('Agent')];
    const g = ledgerGlyphs(tools);
    expect(g.glyphs.map(x => x.icon)).toEqual(['fileText', 'terminal', 'bot']);
    expect(g.glyphs[1].status).toBe('error');
    expect(g.more).toBe(0);
    expect(ledgerGlyphs(Array.from({ length: 20 }, () => call('Read')), 14).more).toBe(6);
  });

  it('lists a folded run\'s distinct files and sums its edits', () => {
    const run = [
      call('Edit', { file_path: '/r/a.ts', old_string: 'x', new_string: 'y' }),
      call('Edit', { file_path: '/r/a.ts', old_string: 'p', new_string: 'q\nr' }),
      call('Write', { file_path: '/r/b.md', content: 'hi' }),
      call('Edit', { file_path: '/r/c.ts', old_string: '', new_string: 'z' }),
      call('Edit', { file_path: '/r/d.ts', old_string: '', new_string: 'z' }),
    ];
    const f = groupFiles(run);
    expect(f.files.map(x => x.name)).toEqual(['a.ts', 'b.md', 'c.ts']);
    expect(f.more).toBe(1);
    expect(f.stats).toEqual({ additions: 6, deletions: 2 });
    expect(groupFiles([call('Read', { file_path: '/r/a.md' })]).stats).toBeNull();
  });
});

describe('ledgerDefaultOpen', () => {
  it('keeps a short finished ledger open and folds a long one', () => {
    expect(ledgerDefaultOpen(3, false)).toBe(true);
    expect(ledgerDefaultOpen(LEDGER_OPEN_MAX, false)).toBe(true);
    expect(ledgerDefaultOpen(LEDGER_OPEN_MAX + 1, false)).toBe(false);
    // Always open while a call runs.
    expect(ledgerDefaultOpen(400, true)).toBe(true);
  });
});

describe('WD1: a previewed Read result', () => {
  it('names the full line count the cold open sent, not the preview’s', () => {
    const preview = '     1→a\n     2→b';
    expect(describeRow('Read', { file_path: '/x/a.rs' }, preview, 100).verb).toBe('Read 100 lines');
    expect(describeRow('Read', { file_path: '/x/a.rs' }, preview).verb).toBe('Read 2 lines');
  });
});
