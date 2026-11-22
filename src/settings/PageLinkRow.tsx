/** A row for something that moved off Settings to another page (Settings PR 4,
 *  plan 3.4): its name, one line on where it is, and the button that opens it.
 *  The name is the registered control label, so search and deep links land on
 *  the row. */
export function PageLinkRow(props: { title: string; text: string; button: string; onOpen: () => void }) {
  return (
    <div class="settings-row">
      <div class="settings-row-label">
        <div class="settings-row-k">{props.title}</div>
        <div class="settings-row-v">{props.text}</div>
      </div>
      <button type="button" class="btn" onClick={props.onOpen}>{props.button}</button>
    </div>
  );
}
