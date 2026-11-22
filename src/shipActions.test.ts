import { describe, expect, it } from 'vitest';
import { nextSteps, openPullRequests, outsideClass, readOutsideContent, shipActionsFor, shipOffer, type ShipTool, type ShipTurn } from './shipActions';

const ids = (text: string, tools = []) => shipActionsFor(text, tools, true).map(a => a.id);

describe('openPullRequests', () => {
  it('finds a PR named in the reply', () => {
    expect(openPullRequests('Opened PR #238 on `feat/x`. CI is green.')).toEqual([238]);
    expect(openPullRequests('Draft: https://github.com/acme/app/pull/41')).toEqual([41]);
    expect(openPullRequests('The pull request #7 is ready for review.')).toEqual([7]);
  });

  it('drops a PR the reply says it merged', () => {
    expect(openPullRequests('PR #236 merged `e2b8a986`, CI green.')).toEqual([]);
    expect(openPullRequests('Merged PR #12 into main.')).toEqual([]);
    expect(openPullRequests('PR #12 has been merged.')).toEqual([]);
  });

  it('keeps a PR that is not merged yet', () => {
    expect(openPullRequests('PR #12 is not merged yet.')).toEqual([12]);
    expect(openPullRequests('PR #12 can be merged once CI passes.')).toEqual([12]);
    expect(openPullRequests("PR #12 hasn't been merged.")).toEqual([12]);
    expect(openPullRequests('PR #12 is ready to merge.')).toEqual([12]);
  });

  it('judges each PR by its own part of the sentence', () => {
    expect(openPullRequests('Merged PR #10; PR #11 is open with CI green.')).toEqual([11]);
    expect(openPullRequests('PR #10 merged. PR #11 waits on review.')).toEqual([11]);
  });

  it('a merge anywhere in the reply wins over an earlier open mention', () => {
    expect(openPullRequests('Opened PR #5.\n\nCI went green, so PR #5 merged as `abc123`.')).toEqual([]);
  });

  it('ignores PRs the reply closes or calls superseded', () => {
    expect(openPullRequests('Close the superseded PRs #178 through #183.')).toEqual([]);
    expect(openPullRequests('PR #9 was closed in favour of the new branch.')).toEqual([]);
  });

  it('ignores bare issue numbers and code blocks', () => {
    expect(openPullRequests('Fixes #44 and #45.')).toEqual([]);
    expect(openPullRequests('```\ngh pr view PR #3\n```\nNothing else to do.')).toEqual([]);
  });

  it('reads gh pr create and gh pr merge tool calls', () => {
    const created = [{ name: 'Bash', input: { command: 'gh pr create --fill' }, result: 'https://github.com/a/b/pull/90\n' }];
    expect(openPullRequests('Done.', created)).toEqual([90]);
    expect(openPullRequests('Done.', [...created, { name: 'Bash', input: { command: 'gh pr merge 90 --merge' }, result: '' }])).toEqual([]);
    expect(openPullRequests('PR #90 is up.', [{ name: 'Bash', input: { command: 'gh pr merge --merge' } }])).toEqual([]);
  });

  it('skips failed tool calls', () => {
    const failedMerge = [{ name: 'Bash', input: { command: 'gh pr merge 90' }, isError: true, result: 'not mergeable' }];
    expect(openPullRequests('PR #90 is open.', failedMerge)).toEqual([90]);
  });
});

describe('shipActionsFor', () => {
  it('offers install when the reply talks about installing', () => {
    expect(ids('PR #238 is open with CI green; once merged I can build and install it.')).toEqual(['merge-install']);
  });

  it('offers deploy when the reply talks about deploying', () => {
    expect(ids('PR #12 is ready. Merging it triggers the Vercel deploy.')).toEqual(['merge-deploy']);
  });

  it('offers both when the reply mentions both', () => {
    expect(ids('PR #3 is open. After merge: install locally, then deploy the API.')).toEqual(['merge-install', 'merge-deploy']);
  });

  it('falls back to Merge alone', () => {
    expect(ids('PR #3 is open and CI is green.')).toEqual(['merge']);
  });

  it('offers nothing without an open PR', () => {
    expect(ids('Installed `c76d2075`; provenance clean.')).toEqual([]);
  });

  it('after a merge, offers the install or deploy the reply says is still to do', () => {
    expect(ids('I merged the fix as PR #247 (CI green). It isn’t installed yet: the app is still on `0f859d4c`.')).toEqual(['install']);
    expect(ids('Merged PR #9. Do you want me to build and install `e178cbdf`?')).toEqual(['install']);
    expect(ids('Merged PR #9; it hasn’t been deployed.')).toEqual(['deploy']);
    expect(ids('Merged PR #9 and installed it; provenance clean.')).toEqual([]);
    expect(ids('Merged PR #9.')).toEqual([]);
  });
});

describe('shipOffer', () => {
  const reply: ShipTurn[] = [
    { role: 'user', text: 'fix it' },
    { role: 'assistant', text: 'Working.', tools: [{ name: 'Bash', input: { command: 'gh pr create' }, result: 'https://github.com/a/b/pull/4' }] },
    { role: 'assistant', text: 'PR #4 is green and ready to install.' },
  ];

  it('reads the whole last reply, tools included', () => {
    expect(shipOffer(reply, { busy: false, queued: false }, null, true).map(a => a.id)).toEqual(['merge-install']);
  });

  it('stays hidden while busy, queued, streaming or live', () => {
    expect(shipOffer(reply, { busy: true, queued: false })).toEqual([]);
    expect(shipOffer(reply, { busy: false, queued: true })).toEqual([]);
    expect(shipOffer([...reply.slice(0, 2), { ...reply[2], partial: true }], { busy: false, queued: false })).toEqual([]);
    expect(shipOffer([...reply.slice(0, 2), { ...reply[2], runId: 'r1' }], { busy: false, queued: false, liveRunId: 'r1' })).toEqual([]);
  });

  it('goes away once the user sends something', () => {
    expect(shipOffer([...reply, { role: 'user', text: 'merge and install' }], { busy: false, queued: false })).toEqual([]);
  });

  it('skips restored-history notices but not other notices', () => {
    expect(shipOffer([...reply, { role: 'system', text: '(older turns)', hydratedNotice: true }], { busy: false, queued: false })).toHaveLength(1);
    expect(shipOffer([...reply, { role: 'system', text: 'Stopped.' }], { busy: false, queued: false })).toEqual([]);
  });

  it('ignores an older reply', () => {
    expect(shipOffer([...reply, { role: 'user', text: 'and now?' }, { role: 'assistant', text: 'All done.' }], { busy: false, queued: false })).toEqual([]);
  });
});

describe('ship offers rest on checked state when Hollow has it (next-level P20)', () => {
  const turns = [{ role: 'user' as const, text: 'ship it' }, { role: 'assistant' as const, text: 'PR #12 is ready to merge and install.' }];
  const idle = { busy: false, queued: false };
  it('a PR GitHub reports merged or closed gets no merge offer', () => {
    expect(shipOffer(turns, idle, { state: 'merged', number: 12 })).toEqual([]);
    expect(shipOffer(turns, idle, { state: 'closed', number: 12 })).toEqual([]);
  });
  it('a check of some other PR says nothing about this one', () => {
    expect(shipOffer(turns, idle, { state: 'merged', number: 11 }, true).map(a => a.id)).toEqual(['merge-install']);
    expect(shipOffer(turns, idle, { state: 'open', number: 11 })[0].basis).toBe('reply');
  });
  it('a merged PR GitHub confirms still gets its install offer', () => {
    const after = [{ role: 'user' as const, text: 'ship it' }, { role: 'assistant' as const, text: 'Merged PR #12. Not installed yet.' }];
    expect(shipOffer(after, idle, { state: 'merged', number: 12 }, true).map(a => a.id)).toEqual(['install']);
  });
  it('labels the basis of the offer', () => {
    expect(shipOffer(turns, idle, { state: 'open', number: 12 })[0].basis).toBe('checked');
    expect(shipOffer(turns, idle, null)[0].basis).toBe('reply');
  });
});

describe("the agent's suggested next message", () => {
  const idle = { busy: false, queued: false };
  const said = (text: string): ShipTurn[] => [{ role: 'user', text: 'fix it' }, { role: 'assistant', text }];

  it('reads up to three lines from the last hollow-next block', () => {
    const text = 'Fixed.\n\n```hollow-next\nRun the native checks\n- Draft the release note\n2. "Open the plan"\nA fourth\n```';
    expect(nextSteps(text).map(a => a.prompt)).toEqual(['Run the native checks', 'Draft the release note', 'Open the plan']);
    expect(nextSteps(text)[0]).toMatchObject({ id: 'next-0', basis: 'agent', label: 'Run the native checks' });
    expect(nextSteps('```hollow-next\nold\n```\nthen\n```hollow-next\nnew\n```').map(a => a.prompt)).toEqual(['new']);
  });

  it('skips blank, duplicate and overlong lines, and replies without a block', () => {
    expect(nextSteps('```hollow-next\n\nShip it\nship it.\n' + 'x'.repeat(121) + '\n```').map(a => a.prompt)).toEqual(['Ship it']);
    expect(nextSteps('```hollow-next\n' + 'y'.repeat(120) + '\n```').map(a => a.prompt)).toEqual(['y'.repeat(120)]);
    expect(nextSteps('No block here.')).toEqual([]);
    expect(nextSteps('```text\nhollow-next\n```')).toEqual([]);
  });

  it('offers it on any finished reply, with no pull request involved', () => {
    expect(shipOffer(said('Drafted.\n```hollow-next\nTighten the intro\n```'), idle).map(a => a.prompt)).toEqual(['Tighten the intro']);
    expect(shipOffer(said('Drafted.\n```hollow-next\nTighten the intro\n```'), { busy: true, queued: false })).toEqual([]);
  });

  it('leads the ship offers, which drop what the agent already said', () => {
    const text = 'Opened PR #12; it needs an install.\n```hollow-next\nMerge and install it\nReview the diff first\n```';
    expect(shipOffer(said(text), idle, null, true).map(a => [a.prompt, a.basis])).toEqual([['Merge and install it', 'agent'], ['Review the diff first', 'agent']]);
    const other = 'Opened PR #12; it needs an install.\n```hollow-next\nReview the diff first\n```';
    expect(shipOffer(said(other), idle, null, true).map(a => a.id)).toEqual(['next-0', 'merge-install']);
  });

  it("drops the agent's merge once GitHub reports the PR merged", () => {
    const text = 'Opened PR #12 for the install fix.\n```hollow-next\nMerge and install\nCheck the logs\n```';
    expect(shipOffer(said(text), idle, { state: 'merged', number: 12 }).map(a => a.prompt)).toEqual(['Check the logs']);
  });
});

describe('suggestions after outside content (durability D2)', () => {
  const t = (name: string, input?: unknown): ShipTool => ({ name, input });
  it('classifies web fetch and search tools across providers', () => {
    for (const name of ['WebFetch', 'WebSearch', 'web_search', 'google_web_search', 'web_fetch', 'fetch_url']) expect(outsideClass(t(name)), name).toBe('web');
    expect(outsideClass(t('Bash', { command: 'curl -s https://example.com/page' }))).toBe('web');
    expect(outsideClass(t('command_execution', { command: 'wget http://example.com' }))).toBe('web');
    expect(outsideClass(t('Bash', { command: 'curl -s localhost:1420' }))).toBeNull();
  });
  it('classifies browser tools, including Hollow\'s own tab readers', () => {
    for (const name of ['mcp__hollow_computer__browser_current_tab', 'mcp__hollow_computer__browser_list_tabs', 'mcp__hollow_computer__search_bookmarks', 'mcp__Claude_in_Chrome__read_page', 'mcp__playwright__browser_snapshot', 'browser_use', 'in_app_browser']) {
      expect(outsideClass(t(name)), name).toBe('browser');
    }
  });
  it('classifies any other MCP server as a connector, in the normalized form every provider uses', () => {
    for (const name of ['mcp__claude_ai_Notion__notion-search', 'mcp__m365__read_file', 'mcp__plaud__get_transcript', 'mcp__github__get_issue']) expect(outsideClass(t(name)), name).toBe('connector');
  });
  it("doesn't count Hollow's own task, app, vault, question and team tools", () => {
    for (const name of ['mcp__hollow_tasks__list_tasks', 'mcp__hollow_app__read_workspace', 'mcp__hollow_vault__search_vault', 'mcp__hollow_questions__ask_user_questions', 'mcp__hollow_team__ask_agent', 'mcp__hollow_computer__open_url', 'mcp__hollow_computer__quit_app']) {
      expect(outsideClass(t(name)), name).toBeNull();
    }
  });
  it("doesn't count local tools or a reply with no tools", () => {
    for (const name of ['Read', 'Edit', 'Grep', 'Bash', 'ToolSearch', 'file_change', 'read_file']) expect(outsideClass(t(name, { command: 'gh pr view 12' })), name).toBeNull();
    expect(readOutsideContent([])).toBe(false);
    expect(readOutsideContent([t('Read'), t('WebFetch')])).toBe(true);
  });
  it('marks every offer of a turn that read outside content for review, and no other', () => {
    const idle = { busy: false, queued: false };
    // No PR here: ship offers always wait for review (P0-01), tested apart.
    const text = 'Drafted the summary.\n```hollow-next\nReview the diff first\nTighten the intro\n```';
    const turns = (tools: ShipTool[]): ShipTurn[] => [{ role: 'user', text: 'go' }, { role: 'assistant', text: 'Looking.', tools }, { role: 'assistant', text }];
    expect(shipOffer(turns([t('mcp__hollow_tasks__list_tasks')]), idle).map(a => a.review)).toEqual([undefined, undefined]);
    expect(shipOffer(turns([t('WebFetch', { url: 'https://example.com' })]), idle).map(a => [a.review, a.reviewWhy])).toEqual([[true, 'outside'], [true, 'outside']]);
    // Only the last reply's turn counts: an earlier turn's web read doesn't.
    const earlier: ShipTurn[] = [{ role: 'user', text: 'a' }, { role: 'assistant', text: 'read', tools: [t('WebSearch')] }, ...turns([])];
    expect(shipOffer(earlier, idle).every(a => !a.review)).toBe(true);
  });
  it('drops suggestions carrying control or invisible formatting characters', () => {
    const block = (line: string) => nextSteps('```hollow-next\n' + line + '\nKeep this\n```').map(a => a.prompt);
    for (const bad of ['Send\u0007 it', 'Merge\u202e lla', 'Zero\u200bwidth', 'Tab\tinside', 'Line\u2028sep']) expect(block(bad), JSON.stringify(bad)).toEqual(['Keep this']);
  });
});

describe('ship offers need a second Enter (P0-01)', () => {
  const idle = { busy: false, queued: false };
  const said = (text: string): ShipTurn[] => [{ role: 'user', text: 'fix it' }, { role: 'assistant', text }];
  it('marks every ship offer for review, in a Hollow build workspace or not', () => {
    for (const hollow of [true, false]) {
      expect(shipOffer(said('PR #12 is open and ready to install.'), idle, null, hollow).every(a => a.review === true), `hollow ${hollow}`).toBe(true);
      expect(shipOffer(said('PR #3 is open. After merge: install locally, then deploy the API.'), idle, null, hollow).every(a => a.review === true)).toBe(true);
      expect(shipOffer(said('Merged PR #9; it hasn\u2019t been deployed.'), idle, null, hollow).map(a => [a.id, a.review])).toEqual([['deploy', true]]);
    }
    expect(shipOffer(said('Merged PR #9. Not installed yet.'), idle, null, true).map(a => [a.id, a.review])).toEqual([['install', true]]);
  });

  it("marks the agent's own merge, install or deploy lines for review, and leaves the rest", () => {
    const text = 'Done.\n```hollow-next\nMerge and install it\nReview the diff first\nDeploy to staging\n```';
    expect(shipOffer(said(text), idle, null, true).map(a => [a.prompt, a.review ?? false])).toEqual([
      ['Merge and install it', true], ['Review the diff first', false], ['Deploy to staging', true],
    ]);
    for (const line of ['Merging it now', 'Redeploy staging', 'Ship it', 'Land it', 'Release v2', 'Reinstalling is fine']) {
      expect(shipOffer(said('Done.\n```hollow-next\n' + line + '\n```'), idle)[0].review, line).toBe(true);
    }
  });

  it('marks every agent line of a reply that earned a ship offer, "Yes, go ahead" included', () => {
    const text = 'PR #42 is open. Shall I merge and install?\n```hollow-next\nYes, go ahead\nShow me the diff\n```';
    expect(shipOffer(said(text), idle, null, true).every(a => a.review === true)).toBe(true);
  });

  it('offers only Merge outside a Hollow build workspace', () => {
    expect(shipOffer(said('Run `npm install`; PR #12 is open'), idle, null, false).map(a => a.id)).toEqual(['merge']);
    expect(shipOffer(said('PR #12 is open; install it once merged.'), idle, null, false).map(a => a.id)).toEqual(['merge']);
    expect(shipOffer(said('Merged PR #9. Not installed yet.'), idle, null, false)).toEqual([]);
    // Deploy isn't Hollow's install: it is offered anywhere.
    expect(shipOffer(said('PR #12 is ready. Merging it triggers the Vercel deploy.'), idle, null, false).map(a => a.id)).toEqual(['merge-deploy']);
  });

  it('never reads inline code as an install or deploy', () => {
    expect(shipOffer(said('Run `npm install`; PR #12 is open'), idle, null, true).map(a => a.id)).toEqual(['merge']);
    expect(shipOffer(said('PR #12 is open; it adds a `deploy` script.'), idle, null, true).map(a => a.id)).toEqual(['merge']);
    expect(shipOffer(said('PR #12 is open; `npm ci` passes. Merge, then install.'), idle, null, true).map(a => a.id)).toEqual(['merge-install']);
  });
});
