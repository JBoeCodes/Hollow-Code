import { Match, Switch, lazy, onMount, type JSX } from 'solid-js';
import { OutsideSuspense } from './OutsideSuspense';
import { RecoverableView } from './RecoverableView';
import { preloadWhenIdle } from './OnceOpened';
import { StudioHome } from './StudioHome';
import { goToPage } from '../store/workspaces';
import { Button, Tabs } from './ui';
import { WORK_VIEWS, openWorkView, workView, type WorkView } from '../store/destinations';
import type { WorkspaceStarter } from '../studio';
import '../styles/destinations.css';

// Running keeps its own chunk, loaded the first time its tab is chosen.
const ACTIVITY = () => import('./MissionControl');
const MissionControl = lazy(() => ACTIVITY().then(m => ({ default: m.MissionControl })));
// Pull requests (parity 7.3) is a chunk of its own, loaded the first time its tab is chosen.
const GITHUB = () => import('./GithubPages');
const GithubPages = lazy(() => GITHUB().then(m => ({ default: m.GithubPages })));

/**
 * Work: Home starts a conversation or resumes a project. Running is what is
 * running and what just finished (formerly Mission Control). Today, Weekly
 * review and the workspace map were cut (overnight 11-08 S3 and S4).
 */
export function YourWork(props: { onNew: (starter?: WorkspaceStarter) => void }) {
  const choose = (v: WorkView) => { openWorkView(v); };
  const viewItems = WORK_VIEWS.map(v => ({
    value: v.id, label: v.label, hint: v.hint,
  }));
  // Running was an overlay that draws its own loading and error lines; a
  // page Suspense would swap the whole view for a skeleton on every reload
  // (lane B's finding), so it renders outside it. Its chunk loads when the
  // shell is idle, so the tab shows at once.
  onMount(() => preloadWhenIdle([ACTIVITY]));
  // OutsideSuspense's root has no owner, so the page's ErrorBoundary never
  // sees a view's error: each view gets its own, inside that root, which
  // fails in place with the tab's name and Retry (RF-909, run 13 FIX-EB).
  const label = (v: WorkView) => WORK_VIEWS.find(x => x.id === v)!.label;
  const Recoverable = (p: { name: string; children: JSX.Element }) =>
    <RecoverableView name={p.name} compact suspends={false}>{p.children}</RecoverableView>;
  return <div class="your-work" data-view={workView()}>
    <div class="home-view-actions"><Tabs class="destination-views" label="Projects views" items={viewItems} value={workView()} onChange={choose} /><Button size="sm" variant="ghost" icon="checkSquare" onClick={() => void goToPage('tasks')}>Tasks</Button></div>
    <Switch>
      <Match when={workView() === 'home'}><StudioHome onNew={props.onNew} /></Match>
      <Match when={workView() === 'activity'}><OutsideSuspense>{() => <Recoverable name={label('activity')}><MissionControl /></Recoverable>}</OutsideSuspense></Match>
      <Match when={workView() === 'pulls'}><OutsideSuspense>{() => <Recoverable name={label('pulls')}><GithubPages /></Recoverable>}</OutsideSuspense></Match>
    </Switch>
  </div>;
}
