import { describe, expect, it } from 'vitest';
import source from './UpdatesPeople.tsx?raw';
import updates from './Updates.tsx?raw';

describe('Settings, Updates: People card (U9)', () => {
  it('loads lazily and the channel picker offers only allowed channels', () => {
    expect(updates).toContain("lazy(() => import('./UpdatesPeople')");
    expect(updates).toContain("(updateState()?.allowedChannels ?? ['main', 'nightly']).includes(");
    expect(updates).toContain('options={channelOptions()}');
    expect(source).toContain('Connected as {name()}.');
  });
  it('switches publishing through the saved settings', () => {
    expect(source).toContain('await api.updateSetPublisher(enabled)');
    expect(source).toContain('props.onSettings(next)');
    expect(source).toContain('This Mac publishes updates');
  });
  it('lists, adds, changes and revokes on the publisher Mac, one call at a time', () => {
    expect(source).toContain('api.updatePeopleList()');
    expect(source).toContain('api.updatePeopleAdd(name.trim(), channels)');
    expect(source).toContain('api.updatePeopleSetChannels(person.id, next)');
    expect(source).toContain('api.updatePeopleRevoke(person.id)');
    expect(source).toContain('if (busy()) return;');
    expect(source).toContain('up to 90 seconds');
  });
  it('confirms Revoke before the call', () => {
    const revoke = source.slice(source.indexOf('async function revoke('));
    expect(revoke.indexOf('await showConfirm(')).toBeLessThan(revoke.indexOf('api.updatePeopleRevoke('));
    expect(revoke).toContain("confirmLabel: 'Revoke', danger: true");
  });
  it('shows the install link once with Copy and never stores it', () => {
    expect(source).toContain('setAdded({ name: result.person.name, link: result.installLink })');
    expect(source).toContain('navigator.clipboard.writeText(a.link)');
    expect(source).toContain('onCleanup(() => { alive = false; setAdded(null); });');
    expect(source).toContain('Hollow shows it only now');
    expect(source).not.toMatch(/localStorage|sessionStorage|console\.|installLink\}/);
  });
});
