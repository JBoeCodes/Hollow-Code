// Where a Settings target that moved to another page opens (Settings PR 4,
// overnight 11-12): a Library tab (Skills, History, Learnings, Conversations)
// or Automations with its windows and run queue fold. `requestOpenSettingsSection`
// (every deep link, palette row and `settings:` URL), Settings' own jump and
// `openSkills` all end here. The destination modules import `./runtime`, so
// they load on use.
import type { LibraryMode } from './libraryModeState';
import { flushViewTransition } from '../viewTransitions';
import { activePage, setActivePage, type PageId } from './runtime';
import { MOVED_LABELS, resolveSettingsTarget, type SettingsTarget } from '../settingsResolve';

/** Resolves true once the destination was opened, false when a navigation
 *  guard (an unsaved draft answered Keep editing) refused it. */
export async function openSettingsPageTarget(target: { page: string; tab?: string; label?: string }): Promise<boolean> {
  let opened = false;
  const onOpen = () => { opened = true; };
  if (target.page === 'vault' && target.tab) {
    const { openLibraryTab } = await import('./libraryMode');
    await openLibraryTab(target.tab as LibraryMode, onOpen);
  } else if (target.page === 'workflows' && (target.tab === 'windows' || target.tab === 'queue')) {
    const { openAutomationWindows } = await import('./automationsViews');
    await openAutomationWindows(target.tab, onOpen);
  } else { setActivePage(target.page as PageId); onOpen(); }
  // A page jump still waiting for its view transition applies now.
  flushViewTransition();
  // An old link's control (overnight 11-13 S1): the page opens on it, not
  // just on its tab. The tab's own name (Skills) is the tab itself.
  // It looks only while the page it opened stays (Conversations opens the
  // chat page, so the page is read here) and stops on any input; the next
  // deep link, with a label or not, stops one still looking (11-13 review).
  const label = target.label;
  const wanted = opened && label && label !== MOVED_LABELS[label]?.search?.tabLabel && typeof document !== 'undefined';
  if (wanted) {
    const page = activePage();
    landingLoaded = true;
    void import('../settingsJump').then(j => j.landOnPageControl(label, () => activePage() === page));
  } else if (landingLoaded) void import('../settingsJump').then(j => j.cancelPageLanding());
  return opened;
}

/** Whether a landing was ever started, so a later link stops it without loading the module otherwise. */
let landingLoaded = false;

/** The page target a `settings:` link or agent step opens directly, or null
 *  when it goes through Settings: a section, or any target while Settings is
 *  the page showing (its own jump asks about unsaved changes first). */
export function directPageTarget(id: string, control: string | undefined, settingsShown: boolean): Extract<SettingsTarget, { kind: 'page' }> | null {
  const target = resolveSettingsTarget(id, control || undefined);
  return target.kind === 'page' && !settingsShown ? target : null;
}
