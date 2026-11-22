import source from './components/CanvasGallery.tsx?raw';
import { describe, expect, it } from 'vitest';
import ts from 'typescript';

// Ownership regression: the expensive diff is conditional, while the two
// live canvas owners remain outside that condition. Browser checks exercise
// form/iframe identity and revision actions in the end-of-run suite.
const ast = ts.createSourceFile('CanvasGallery.tsx', source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
function find(predicate: (node: ts.Node) => boolean) {
  const matches: ts.Node[] = [];
  const visit = (node: ts.Node) => { if (predicate(node)) matches.push(node); ts.forEachChild(node, visit); };
  visit(ast);
  return matches;
}
describe('canvas comparison ownership', () => {
  it('mounts source changes only inside the selected canvas disclosure', () => {
    const diff = find(n => ts.isJsxSelfClosingElement(n) && n.tagName.getText(ast) === 'DiffView')[0];
    expect(diff).toBeDefined();
    const owner = diff.parent;
    expect(ts.isJsxElement(owner) && owner.openingElement.getText(ast)).toBe('<Show when={sourceOpen()}>');
    expect(owner.getText(ast)).not.toContain('frame(');
    expect(source).toContain("const [sourceOpen, setSourceOpen] = createSignal(false)");
    expect(source).toContain('onToggle={e => setSourceOpen(e.currentTarget.open)}');
    expect(source).toContain('<summary>View source changes</summary>');
  });
  it('labels both rendered sides and keeps revision-specific source actions', () => {
    expect(source).toContain('<figcaption>Before · v{v.before.n}</figcaption>');
    expect(source).toContain('<figcaption>After · v{v.after.n}</figcaption>');
    expect(source).toContain('action: () => openSource(revision)');
  });
});
