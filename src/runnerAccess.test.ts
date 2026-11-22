import { describe, expect, it } from 'vitest';
import { remoteRunNotice, reachText } from './remoteRunNotice';
import { runnerAccessHint } from './components/runnerAccessHint';

// Remote runners (2026-11-03): what a workspace's runner turns may reach,
// beside Runs on and on a turn's start line.
describe('runner access', () => {
  it('says plainly what the turn reaches and why not', () => {
    expect(reachText(null)).toBe('');
    expect(reachText({ github: 'runner-token', repo: 'acme/widgets', cloud: 'limited' })).toBe(
      ' This turn reaches GitHub (acme/widgets) with your runner token; Google Cloud as you, no production changes.',
    );
    expect(reachText({ github: 'gh-login' })).toBe(" This turn reaches GitHub with this Mac's gh login.");
    expect(reachText({ github: 'no-token' })).toContain('no runner token is saved');
    expect(reachText({ cloud: 'service-account', serviceAccount: 'deploy@p.iam.gserviceaccount.com' })).toContain('Google Cloud as deploy@p.iam.gserviceaccount.com');
    expect(reachText({ cloud: 'no-gcloud' })).toContain("gcloud isn't installed on this Mac");
    const start = remoteRunNotice({ type: 'remoteRun', phase: 'start', host: 'runner', access: { github: 'runner-token', cloud: 'full' } });
    expect(start).toContain('Tools, History and Stop stay on this Mac. This turn reaches GitHub with your runner token; Google Cloud as you.');
    expect(remoteRunNotice({ type: 'remoteRun', phase: 'start', host: 'runner', scope: 'folder', access: { cloud: 'limited' } })).toContain('no production changes');
  });

  it('describes the On the runner choice', () => {
    expect(runnerAccessHint({ github: false, cloud: 'off' })).toBe('Runner turns reach only their own checkout: no GitHub, no Google Cloud.');
    expect(runnerAccessHint({ github: true, cloud: 'limited' })).toContain('a guard rail rather than a hard limit');
    expect(runnerAccessHint({ github: true, cloud: 'off' })).toContain('Credentials stay on this Mac');
    expect(runnerAccessHint({ github: false, cloud: 'service-account' })).toBe('Name the service account to use. Credentials stay on this Mac and reach the runner one call at a time.');
    expect(runnerAccessHint({ github: false, cloud: 'service-account', serviceAccount: 'd@p.iam.gserviceaccount.com' })).toContain("the account's own permissions are the hard limit");
  });
});
