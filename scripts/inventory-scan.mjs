// The review's inventory method (docs/review-2026-10-13/generate-inventory.mjs),
// run on the current source without writing anything (plan completion L4, N1):
// every interactive JSX control and every action-shaped object literal under
// src/, with its file, line and label. scripts/inventory-reconcile.mjs and
// src/inventoryMap.test.ts join it with the classification in
// src/inventoryMap.ts. Static evidence, like the review's.
import fs from 'node:fs';
import path from 'node:path';
import ts from 'typescript';

const flat = s => s.replace(/\s+/g, ' ').trim();
const walk = d => fs.readdirSync(d, { withFileTypes: true }).flatMap(e => e.isDirectory() ? walk(path.join(d, e.name)) : [path.join(d, e.name)]);
const INTERACTIVE = /^(button|input|textarea|select|option|summary|a|Button|IconButton|Select|Switch|Menu|ActionMenu|ContextMenu|Segmented|ListRow|ResizeHandle|DictateButton|MoreButton)$/;

/** `{ controls, actions }` for the source under `root/src`. */
export function scanInventory(root) {
  const files = walk(path.join(root, 'src')).filter(f => /\.tsx?$/.test(f) && !f.endsWith('.test.ts') && !f.includes(`${path.sep}generated${path.sep}`));
  const controls = [], actions = [];
  for (const abs of files) {
    const f = path.relative(root, abs).split(path.sep).join('/');
    const text = fs.readFileSync(abs, 'utf8');
    const sf = ts.createSourceFile(f, text, ts.ScriptTarget.Latest, true, f.endsWith('.tsx') ? ts.ScriptKind.TSX : ts.ScriptKind.TS);
    const line = n => sf.getLineAndCharacterOfPosition(n.getStart(sf)).line + 1;
    const val = n => !n ? '' : ts.isStringLiteral(n) ? n.text : ts.isJsxExpression(n) ? n.expression?.getText(sf) ?? '' : n.getText(sf);
    const visit = n => {
      if (ts.isJsxOpeningElement(n) || ts.isJsxSelfClosingElement(n)) {
        const tag = n.tagName.getText(sf);
        const attrs = Object.fromEntries(n.attributes.properties.filter(ts.isJsxAttribute).map(a => [a.name.getText(sf), val(a.initializer) || 'true']));
        const events = Object.keys(attrs).filter(k => /^on[A-Z]|^on:/.test(k));
        if (INTERACTIVE.test(tag) || events.length || ['button', 'menuitem', 'tab', 'checkbox', 'switch', 'slider'].includes(attrs.role)) {
          controls.push({ file: f, line: line(n), element: tag, label: flat(attrs['aria-label'] || attrs.label || attrs.title || attrs.placeholder || attrs.name || ''), events });
        }
      }
      if (ts.isObjectLiteralExpression(n)) {
        const props = Object.fromEntries(n.properties.filter(ts.isPropertyAssignment).map(p => [p.name.getText(sf), p.initializer]));
        if ((props.label || props.title) && (props.action || props.run || props.default || (props.id && props.key))) {
          actions.push({ file: f, line: line(n), label: val(props.label || props.title), key: val(props.id) });
        }
      }
      ts.forEachChild(n, visit);
    };
    visit(sf);
  }
  return { controls, actions };
}
