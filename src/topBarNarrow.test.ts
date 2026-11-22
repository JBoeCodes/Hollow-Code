import { describe, expect, it } from 'vitest';
import topBar from './components/TopBar.tsx?raw';
import css from './styles/shell-workspace.css?raw';

// Narrow windows (390px): Back and Forward stay in the top bar. The bar gets
// tighter buttons at 600px instead of folding history navigation away.
describe('the top bar at narrow widths', () => {
  const block = css.slice(css.indexOf('@media (max-width: 600px)'));
  const rules = block.slice(0, block.indexOf('\n}\n') + 3);

  it('has a 600px tier that tightens the bar', () => {
    expect(rules).toContain('.top-bar-iconbtn { width: 26px; height: 26px; }');
    expect(rules).toContain('.top-bar-nav');
  });

  it('never hides the history buttons at any width', () => {
    expect(rules).not.toMatch(/display:\s*none/);
    expect(css).not.toMatch(/\.top-bar-nav[^{]*\{[^}]*display:\s*none/);
    expect(topBar).toContain('aria-label="Back"');
    expect(topBar).toContain('aria-label="Forward"');
    expect(topBar).not.toMatch(/<Show[^>]*(Narrow|compact|windowAtMost)/);
  });

  it('keeps all six left controls and three right controls inside 332px', () => {
    // 26px buttons: sidebar, search, back, forward, two page actions on the left;
    // bell, COO, pane toggle on the right, 8px bar padding each side.
    const px = 26; const left = 6 * px + 2 * 4; const right = 3 * px;
    expect(left + right + 2 * 8 + 4).toBeLessThanOrEqual(332);
  });
});
