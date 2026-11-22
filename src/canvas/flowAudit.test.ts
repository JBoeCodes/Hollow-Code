import { describe, expect, it } from 'vitest';
import { auditFlow, autoAuditFlow } from './flowAudit';
import { flowsInRoot } from './canvasFlows';
import { canvasLayoutRequest } from './chatCanvas';
import type { El } from './canvasModel';

const release = '[Build]\nsource: Change ready {start}\nchecks: Local gate {info}\n[Review]\nreview: Review findings? {decision}\nfix: Repair findings {warning}\n[Delivery]\nmerge: Merge {success}\nready: Native checks {end}\n[]\nsource -> checks -> review\nreview -> fix: needs work\nfix -> checks: recheck\nreview -> merge: green {active}\nmerge -> ready {active}\nsource -> ready: overview';
describe('flow audit', () => {
  it('reports crossings of ungrouped back links', () => {
    const findings = auditFlow('a -> b -> c\nc -> a\na -> d');
    expect(findings.some(f => f.kind === 'crosses-node')).toBe(true);
  });
  it('keeps the release gallery clean in both authored directions', () => {
    expect(auditFlow(release, 'down')).toEqual([]);
    expect(auditFlow(release, 'right')).toEqual([]);
  });
  it('reports crowded labels in the connection list, bounded and deterministic', () => {
    const lines = Array.from({length:8}, (_, i) => `[G${i}]\nn${i}`);
    for (let a = 0; a < 8; a++) for (let b = 0; b < 8; b++) if (a !== b) lines.push(`n${a} -> n${b}: Connection ${a} to ${b}`);
    const source = lines.join('\n'), result = auditFlow(source);
    expect(result.some(f => f.kind === 'label-listed')).toBe(true);
    expect(result.length).toBeLessThanOrEqual(21);
    expect(auditFlow(source)).toEqual(result);
  });
  it('reports shortened node/link labels and items beyond kit limits', () => {
    expect(auditFlow('a: Wait…')).toEqual([]);
    expect(auditFlow(`a: ${'a'.repeat(70)}`)[0].kind).toBe('node-cut');
    expect(auditFlow(`a -> b: ${'yes '.repeat(20)}`).some(f => f.kind === 'link-cut')).toBe(true);
    expect(auditFlow(Array.from({length:61}, (_, i) => `n${i}`).join('\n')).some(f => f.kind === 'not-shown')).toBe(true);
  });
  it('never routes large flows automatically, including parser overflow', () => {
    expect(autoAuditFlow(Array.from({length:40}, (_, i) => `n${i}`).join('\n'))).toBe(false);
    expect(autoAuditFlow(Array.from({length:61}, (_, i) => `n${i}`).join('\n'))).toBe(false);
    expect(autoAuditFlow('a -> b')).toBe(true);
  });
});
const el = (tagName: string, textContent = '', children: El[] = [], attrs: Record<string, string> = {}): El => ({ tagName, textContent, children, getAttribute: name => attrs[name] ?? null });
it('extracts eight authored flows, skipping inert subtrees and normalizing direction', () => {
  const root = el('HC-CANVAS', '', [el('SCRIPT', '', [el('HC-FLOW', 'bad')]), el('TEMPLATE', '', [el('HC-FLOW', 'bad')]),
    el('DIV', '', Array.from({length:10}, (_, i) => el('HC-FLOW', `n${i}`, [], { direction: 'RIGHT', label: `Flow ${i}` })))]);
  const result = flowsInRoot(root);
  expect(result).toHaveLength(8);
  expect(result[0]).toEqual({ label: 'Flow 0', direction: 'right', text: 'n0' });
  expect(flowsInRoot(el('hc-flow', 'a -> b'))[0].direction).toBe('down');
});
it('drafts a bounded plain correction with its identity and instructions intact', () => {
  const text = canvasLayoutRequest({id:'release',title:'Release <b>plan</b>'}, 'Review <flow>', Array.from({length:21}, () => ({text:'<bad> '.repeat(100)})));
  expect(text).toContain('(id release)');
  expect(text).toContain('Release bplan/b');
  expect(text).toContain('same id');
  expect(text).toContain('1. bad');
  expect(text.length).toBeLessThanOrEqual(2000);
  expect(text).not.toContain('<');
});
