import { expect, it } from 'vitest';
import { projectNameProblem } from './projectCreation';
it('requires a bounded one-line project name and catches case-insensitive duplicates', () => {
  expect(projectNameProblem('  ', [])).toContain('name');
  expect(projectNameProblem('a'.repeat(101), [])).toContain('100');
  expect(projectNameProblem('new\nproject', [])).toContain('one line');
  expect(projectNameProblem('porTAL ', ['Portal'])).toContain('already exists');
  expect(projectNameProblem('Union Rec', ['Portal'])).toBe('');
  expect(projectNameProblem('研究', [])).toBe('');
});
