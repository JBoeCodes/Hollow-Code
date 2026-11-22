import { JSX, Show } from 'solid-js';

/** The one page header (frontier F3): a 20px title, an optional quiet meta
 * line, and actions on the right. No eyebrow, no tagline. Styles in
 * `styles/ui.css`. Render it as a sibling of the page's scroll container. */
export function PageHeader(props: {
  title: JSX.Element;
  meta?: JSX.Element;
  actions?: JSX.Element;
  /** Extra classes on the header (e.g. `work-header`, which specs key on). */
  class?: string;
}) {
  return (
    <header class={`page-header${props.class ? ` ${props.class}` : ''}`}>
      <div class="page-title-block">
        <h1 class="page-title">{props.title}</h1>
        <Show when={props.meta}><div class="page-meta">{props.meta}</div></Show>
      </div>
      <Show when={props.actions}><div class="page-actions">{props.actions}</div></Show>
    </header>
  );
}
