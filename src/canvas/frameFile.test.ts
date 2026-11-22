import { describe, expect, it } from 'vitest';
import { canvasFilePath } from './frameFile';

const WS = '/vault/workspaces/w1';
const ROOT = '/vault';

describe('canvasFilePath (hc-file → the file a canvas link opens)', () => {
  it('resolves a relative path against the conversation folder', () => {
    expect(canvasFilePath('outputs/report.md', WS, ROOT)).toBe('/vault/workspaces/w1/outputs/report.md');
    expect(canvasFilePath('./PLAN.md', WS, ROOT)).toBe('/vault/workspaces/w1/PLAN.md');
  });
  it('keeps the line suffixes and encoding agents write working', () => {
    expect(canvasFilePath('outputs/My%20Report.md:12', WS, ROOT)).toBe('/vault/workspaces/w1/outputs/My Report.md');
  });
  it('allows climbing within the vault, and absolute paths inside the folder or vault', () => {
    expect(canvasFilePath('../../notes/a.md', WS, ROOT)).toBe('/vault/notes/a.md');
    expect(canvasFilePath('/vault/notes/a.md', WS, ROOT)).toBe('/vault/notes/a.md');
    expect(canvasFilePath('/repo/src/a.ts', '/repo', ROOT)).toBe('/repo/src/a.ts');
  });
  it('refuses anything outside the folder and the vault', () => {
    expect(canvasFilePath('/etc/passwd', WS, ROOT)).toBeNull();
    expect(canvasFilePath('../../../etc/passwd', WS, ROOT)).toBeNull();
    expect(canvasFilePath('/vaultx/a.md', WS, ROOT)).toBeNull();
    expect(canvasFilePath('~/secrets.txt', WS, ROOT)).toBeNull();
  });
  it('refuses links that are not files, and the folder itself', () => {
    expect(canvasFilePath('https://example.com/a.md', WS, ROOT)).toBeNull();
    expect(canvasFilePath('javascript:alert(1)', WS, ROOT)).toBeNull();
    expect(canvasFilePath('//host/share/a.md', WS, ROOT)).toBeNull();
    expect(canvasFilePath('#top', WS, ROOT)).toBeNull();
    expect(canvasFilePath('', WS, ROOT)).toBeNull();
    expect(canvasFilePath('.', WS, ROOT)).toBeNull();
    expect(canvasFilePath('../..', WS, ROOT)).toBeNull();
  });
  it('without a base, only absolute paths in the vault open; without either, nothing does', () => {
    expect(canvasFilePath('outputs/a.md', null, ROOT)).toBe('/vault/outputs/a.md');
    expect(canvasFilePath('/vault/a.md', null, ROOT)).toBe('/vault/a.md');
    expect(canvasFilePath('outputs/a.md', null, null)).toBeNull();
    expect(canvasFilePath('/vault/a.md', null, null)).toBeNull();
  });
});
