import { DirectorySetup } from './components/DirectorySetup';

/// First-run window root: what main.tsx renders instead of <App /> when
/// setup_status says no config loaded. Storeless by design: choosing the one
/// Hollow folder (Projects + Vault) is the whole setup; accounts and
/// connectors are added later in Settings, Connectors. The welcome tour
/// was cut (overnight 11-08 S4).
export function OnboardingRoot() {
  return <main class="directory-onboarding"><DirectorySetup firstRun /></main>;
}
