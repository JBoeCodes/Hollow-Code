import { expect, it } from 'vitest';
import { transcriptWarning } from './transcriptWarning';
import { cancelledNotice } from './chatStop';
import type { ChatTurn } from './chatTurnTypes';
const row = (text: string, role: ChatTurn['role'] = 'system'): ChatTurn => ({ role, text, tools: [] });

it('groups exact repeated causes only within one user turn', () => {
  const turns = [row('MCP failed: alpha'), row('MCP failed: beta'), row('MCP failed: alpha'), row('Again', 'user'), row('MCP failed: alpha')];
  expect(transcriptWarning(turns, 0)?.count).toBe(2);
  expect(transcriptWarning(turns, 1)?.count).toBe(1);
  expect(transcriptWarning(turns, 2)?.count).toBe(0);
  expect(transcriptWarning(turns, 4)?.count).toBe(1);
});
it('keeps ordinary notices and agent words out of the warning treatment', () => {
  expect(transcriptWarning([row('Started fresh')], 0)).toBeNull();
  expect(transcriptWarning([row('A test failed', 'assistant')], 0)).toBeNull();
  expect(transcriptWarning([{ ...row('Quota limit'), notice: 'accountSwitch' }], 0)).toBeNull();
});
it('caps the summary without changing the source detail', () => {
  const source = row(`Warning: ${'a'.repeat(200)}\nComplete detail`);
  expect(transcriptWarning([source], 0)?.title).toHaveLength(120);
  expect(source.text).toContain('Complete detail');
});
it('discloses a standalone host error without swallowing an answer or tool call', () => {
  expect(transcriptWarning([row('(error: MCP server failed)', 'assistant')], 0)?.count).toBe(1);
  expect(transcriptWarning([row('Answer\n(error: MCP server failed)', 'assistant')], 0)).toBeNull();
  expect(transcriptWarning([{ ...row('(error: Tool failed)', 'assistant'), tools: [{ name: 'Read' }] }], 0)).toBeNull();
});
it('keeps a stop notice neutral even when it says Hollow could not confirm', () => {
  expect(transcriptWarning([row(cancelledNotice('unconfirmed'))], 0)).toBeNull();
  expect(transcriptWarning([row(cancelledNotice('verified'))], 0)).toBeNull();
});
