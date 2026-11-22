import { describe, expect, it } from 'vitest';
import css from './styles/motion-kit.css?raw';
import toolCall from './components/ToolCall.tsx?raw';

describe('motion kit callers (parity 6.1)', () => {
  it('lets a long running-tool verb clip with the ledger ellipsis instead of vanishing', () => {
    expect(css).toMatch(/\.toolcall-verb \.motion-cue \{[^}]*overflow: hidden; text-overflow: ellipsis/);
  });

  it('shimmers a running tool and animates its body', () => {
    expect(toolCall).toContain('<MotionCue kind="tool" active={status() === \'running\'}>');
    expect(toolCall).toContain('<AnimatedHeight><Show when={props.expanded && hasBody()}>');
  });
});
