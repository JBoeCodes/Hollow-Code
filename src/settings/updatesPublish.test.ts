import { describe, expect, it } from 'vitest';
import source from './UpdatesPublish.tsx?raw';
import updates from './Updates.tsx?raw';

describe('Settings, Updates: Publish to Main card (U8)', () => {
  it('loads lazily and only on the publisher Mac', () => {
    expect(updates).toContain("lazy(() => import('./UpdatesPublish')");
    expect(updates).toContain('<Show when={savedSettings()?.publisher}><UpdatesPublish /></Show>');
  });
  it('asks for status on open, Refresh and a pick, never on a timer', () => {
    expect(source).toContain('void load(null);');
    expect(source).toContain('onClick={() => void load()}>Refresh</button>');
    expect(source).toContain('onChange={v => { setOutcome(null); void load(v); }}');
    expect(source).not.toMatch(/setInterval|setTimeout/);
    expect(source).toContain('<Spinner /> Asking the release builder.');
  });
  it('confirms Publish and binds the commit and Main the card showed', () => {
    const publish = source.slice(source.indexOf('async function publish()'), source.indexOf('async function withdraw()'));
    expect(publish.indexOf('await showConfirm(')).toBeLessThan(publish.indexOf('api.updatePublishMain('));
    expect(publish).toContain('api.updatePublishMain(b.version, b.commit!, s.main?.version ?? null)');
    expect(source).toContain('!selected()!.newerThanMain');
  });
  it('confirms Withdraw as a danger and passes the Main it showed', () => {
    const withdraw = source.slice(source.indexOf('async function withdraw()'));
    expect(withdraw.indexOf('await showConfirm(')).toBeLessThan(withdraw.indexOf('api.updateWithdrawMain('));
    expect(withdraw).toContain("confirmLabel: 'Withdraw', danger: true");
    expect(withdraw).toContain('api.updateWithdrawMain(s.main!.version)');
  });
  it('reports every outcome and the history warning', () => {
    for (const outcome of ['published', 'withdrawn', 'refused']) expect(source).toContain(`case '${outcome}'`);
    expect(source).toContain('value.historyWarning');
  });
});
