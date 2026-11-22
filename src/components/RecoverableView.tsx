import { ErrorBoundary, Suspense, createSignal, type JSX } from 'solid-js';
import { StatePanel } from './ui';
import { errorDetails } from '../errorDetails';
import '../styles/recovery.css';

function Recovery(props: { name: string; error: unknown; reset: () => void; compact?: boolean }) {
  const [copyState, setCopyState] = createSignal('');
  async function copy() {
    try {
      await navigator.clipboard.writeText(`${props.name} could not load\n${errorDetails(props.error)}`);
      setCopyState('Details copied.');
    } catch { setCopyState('Could not copy details.'); }
  }
  return <StatePanel kind="failed" class="view-recovery" compact={props.compact}
    title={`${props.name} could not load`} error={props.error}
    actions={<><button type="button" class="btn" onClick={props.reset}>{props.compact ? 'Retry' : 'Reload page'}</button><button type="button" class="btn ghost" onClick={() => void copy()}>Copy details</button><span role="status">{copyState()}</span></>} />;
}

/** Mount inside the page/section owner so navigation disposes failures too.
 *  `suspends={false}` keeps only the error boundary, for a view that must
 *  never sit inside a Suspense boundary: Your work (pages/Work.tsx), whose
 *  side panel reads resources on every pick and would otherwise be swapped
 *  out for the fallback each time, losing focus (overnight 12 A0b); the
 *  old Conversation page's side panel and Your work's tabs, each inside its
 *  OutsideSuspense root, use it the same way. */
export function RecoverableView(props: { name: string; compact?: boolean; fallback?: JSX.Element; suspends?: boolean; children: JSX.Element }) {
  return <ErrorBoundary fallback={(error, reset) => <Recovery name={props.name} compact={props.compact} error={error} reset={reset} />}>
    {props.suspends === false ? props.children : <Suspense fallback={props.fallback}>{props.children}</Suspense>}
  </ErrorBoundary>;
}
