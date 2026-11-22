/// Inline loading indicator — a small ember ring (conic amber arc).
/// Pure-CSS animation; the JSX is just a styled span so every loading
/// surface in the app reads the same.
///
/// Usage:
///   <Spinner /> loading…
///   <span><Spinner /> syncing inbox</span>
export function Spinner(props: { class?: string }) {
  return (
    <span
      class={`spinner-ring ${props.class ?? ''}`}
      role="status"
      aria-label="loading"
    />
  );
}
