import { Show, createEffect, createMemo } from 'solid-js';

/** A bounded source preview with one highlighted range, without creating a
 * DOM element for every line of a potentially 1.5 MB file. */
export default function PreviewSourceLines(props: { text: string; line: number; end?: number; truncated?: boolean }) {
  let mark: HTMLElement | undefined;
  const parts = createMemo(() => {
    const lines = props.text.split('\n');
    if (lines.at(-1) === '') lines.pop();
    const count = Math.max(1, lines.length);
    const start = Math.min(props.line, count);
    const end = Math.min(Math.max(props.end ?? props.line, start), count);
    return { start, end, count, before: lines.slice(0, start - 1).join('\n'), selected: lines.slice(start - 1, end).join('\n'), after: lines.slice(end).join('\n') };
  });
  createEffect(() => {
    parts();
    // Solid runs effects after the range's DOM updates. No delayed callback
    // may scroll a newer file after this target has been replaced.
    mark?.scrollIntoView({ block: 'center', behavior: 'instant' });
  });
  return <>
    <Show when={props.line > parts().count || (props.end ?? props.line) > parts().count}>
      <p class="hint chat-preview-hint">The cited line is beyond {props.truncated ? 'this preview' : 'the current file'} ({parts().count} lines). Showing the last available line.</p>
    </Show>
    <pre class="chat-preview-source">{parts().before}{parts().start > 1 ? '\n' : ''}<mark ref={mark} class="receipt-source-target" data-line={parts().start} data-end={parts().end} aria-label={`Lines ${parts().start} to ${parts().end}`}>{parts().selected || ' '}</mark>{parts().end < parts().count ? '\n' : ''}{parts().after}</pre>
  </>;
}
