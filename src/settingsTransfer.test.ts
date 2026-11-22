import { describe, expect, it } from 'vitest';
import { appliedText, exportFileName, exportedText, nothingToImportText, previewBody } from './settingsTransfer';
import type { SettingsImportPreview } from './types';
import SETTINGS from './pages/Settings.tsx?raw';
import STORE from './store/settingsTransfer.ts?raw';
import API from './api.ts?raw';

const preview: SettingsImportPreview = {
  changes: [
    { key: 'theme', summary: 'The colour theme.', old: 'light', new: 'dark', asks: false },
    { key: 'agents.defaultAgent', summary: 'The agent new chats use.', old: 'claude', new: 'codex', asks: false },
  ],
  unchanged: 37,
  ignored: [{ key: 'jev.apiKey', why: 'a secret: never imported from a file' }],
  exportedAt: '2026-10-09T22:15:00Z',
  hash: 'h',
};

describe('settings export/import words', () => {
  it('names the file by local date', () => {
    expect(exportFileName(new Date(2026, 9, 10, 23, 59))).toBe('hollow-settings-2026-10-10.json');
  });

  it('says what was exported and that secrets never travel', () => {
    expect(exportedText({ path: '/Users/x/h.json', count: 42, skipped: [] }))
      .toBe('Exported 42 settings to /Users/x/h.json. Secrets and sign-ins are never included.');
    expect(exportedText({ path: '/p.json', count: 1, skipped: ['theme'] })).toContain('1 setting couldn’t be read');
  });

  it('previews every change old → new and what is left alone', () => {
    expect(previewBody(preview)).toBe([
      '2 settings will change; 37 already match (file exported 2026-10-09).',
      '',
      '• theme: light → dark',
      '• agents.defaultAgent: claude → codex',
      '',
      'Left alone (1):',
      '• jev.apiKey: a secret: never imported from a file',
    ].join('\n'));
  });

  it('lists what agents may do, background spend and accounts first, saying what each is', () => {
    const body = previewBody({ ...preview, changes: [
      { key: 'heartbeat.enabled', summary: 'The heartbeat runs background agent passes (they spend).', old: 'false', new: 'true', asks: true },
      ...preview.changes,
    ] });
    expect(body.split('\n').slice(0, 5)).toEqual([
      '3 settings will change; 37 already match (file exported 2026-10-09).',
      '',
      'These change what agents may do, what runs on its own, or accounts:',
      '• heartbeat.enabled: false → true (The heartbeat runs background agent passes (they spend))',
      '',
    ]);
  });

  it('says when there is nothing to import, and how an import went', () => {
    expect(nothingToImportText({ ...preview, changes: [] })).toBe('Nothing to import: every setting in the file already matches. 1 entry left alone.');
    expect(appliedText({ applied: 2, failed: [] })).toBe('Imported: 2 settings changed.');
    expect(appliedText({ applied: 1, failed: [{ key: 'theme', why: 'needs the window' }] }))
      .toBe('× Imported 1 setting; 1 setting failed (theme: needs the window).');
  });
});

describe('settings export/import wiring', () => {
  it('Settings → Advanced has both buttons', () => {
    expect(SETTINGS).toContain('onClick={() => void exportSettings()}>Export settings…</button>');
    expect(SETTINGS).toContain('onClick={() => void importSettings()}>Import settings…</button>');
  });

  it('applies only after the confirm, bound to the preview hash', () => {
    const confirm = STORE.indexOf('await showConfirm(');
    const apply = STORE.indexOf('api.settingsImportApply(path, preview.hash)');
    expect(confirm).toBeGreaterThan(0);
    expect(apply).toBeGreaterThan(confirm);
    expect(STORE).toContain('if (!ok) return;');
    expect(API).toContain("invoke<SettingsImportApplied>('settings_import_apply', { path, expectedHash })");
  });
});
