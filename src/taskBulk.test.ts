import { describe, expect, it } from 'vitest';
import { originWorkspace, workOnPrompt } from './taskBulk';
import TASK_DETAIL from './pages/tasks/TaskDetail.tsx?raw';
// The Tasks page (rewritten as an issue list) keeps "Work on this" in the local task detail pane.
const TASKS = TASK_DETAIL;
import CHAT from './components/ChatBody.tsx?raw';

describe('task origin and "Work on this"', () => {
  it('finds a task’s workspace only from an explicit origin, never its name', () => {
    expect(originWorkspace({ origin: 'plan:workspaces/local-q3-review/PLAN.md' })).toBe('local-q3-review');
    expect(originWorkspace({ origin: 'workspace:work-1' })).toBe('work-1');
    expect(originWorkspace({ origin: 'meeting:abc' })).toBeNull();
    expect(originWorkspace({ origin: null })).toBeNull();
    expect(workOnPrompt({ title: 'Revise', vaultPath: 'tasks/r.md', doneWhen: 'Finance signs off', due: '2026-10-01', description: '' })).toContain('Done when: Finance signs off');
  });
});

describe('"Work on this" keeps the draft (morning M5)', () => {
  it('adds the task after the draft, only once the workspace is showing', () => {
    // No DOM harness here: the wiring is pinned by source; addToDraft's own
    // behaviour is tested in chatQueue.test.ts.
    expect(TASKS).not.toMatch(/setChatInput\(workOnPrompt/);
    expect(TASKS).toContain('if(!await selectWorkspaceOpened(wid)||workspaceId()!==wid){');
    expect(TASKS).toContain('setChatInput(addToDraft(chatInput(),workOnPrompt(t)))');
    expect(TASKS.match(/openWithTask\(/g)?.length).toBe(2);
    // Branch, agent-action prefill and the run record's Ask to rerun all append.
    expect(CHAT.match(/setChatInput\(addToDraft\(chatInput\(\), text\)\)/g)?.length).toBe(3);
  });
});
