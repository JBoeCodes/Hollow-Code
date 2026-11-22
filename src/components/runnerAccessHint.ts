import type { RunnerAccess } from '../types';

/** The line beside Runs on → On the runner (remote runners, 2026-11-03):
 *  what this workspace's runner turns may reach, in words. */
export function runnerAccessHint(access: RunnerAccess): string {
  const github = access.github
    ? 'GitHub: push and pull requests, with the credential in Settings → Devices → Runner GitHub access.'
    : '';
  const cloud = access.cloud === 'limited'
    ? 'Google Cloud as you; production changes (Cloud Build approvals, Cloud Run deploys and traffic) are refused there, a guard rail rather than a hard limit.'
    : access.cloud === 'full'
      ? 'Google Cloud as you, production changes included.'
      : access.cloud === 'service-account'
        ? access.serviceAccount
          ? `Google Cloud as ${access.serviceAccount}; production changes are refused there too, and the account's own permissions are the hard limit.`
          : 'Name the service account to use.'
        : '';
  const parts = [github, cloud].filter(Boolean);
  return parts.length
    ? `${parts.join(' ')} Credentials stay on this Mac and reach the runner one call at a time.`
    : 'Runner turns reach only their own checkout: no GitHub, no Google Cloud.';
}
