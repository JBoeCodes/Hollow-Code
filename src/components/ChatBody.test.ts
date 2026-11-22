import { describe, expect, it } from 'vitest';
import chatBodySource from './ChatBody.tsx?raw';

describe('ChatBody reply memoization', () => {
  it('memoizes reply once per turn instead of recomputing replyText on every footer prop', () => {
    expect(chatBodySource).toMatch(/const tail = createMemo\(\(\) => endsReply\(chatTurns, turnIdx\(\)\)/);
    expect(chatBodySource).toMatch(/const reply = createMemo\(\(\) => \(tail\(\) \? \{ \.\.\.t, text: replyText\(chatTurns, turnIdx\(\)\) \} : t\)\);/);
  });
});
