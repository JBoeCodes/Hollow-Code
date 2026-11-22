import { describe, expect, it } from 'vitest';
import { EditorState, type TransactionSpec } from '@codemirror/state';
import { history, undoDepth } from '@codemirror/commands';
import { externalComposerDoc, syncComposerDocument } from './composerDocument';

describe('controlled composer documents', () => {
  function host(doc: string) {
    const extensions = [history()];
    const annotations: boolean[] = [];
    const view = {
      state: EditorState.create({ doc, extensions }),
      setState(state: EditorState) { this.state = state; },
      dispatch(spec: TransactionSpec) {
        const tr = this.state.update(spec);
        annotations.push(!!tr.annotation(externalComposerDoc));
        this.state = tr.state;
      },
    };
    return { view, extensions, annotations };
  }
  it('does not turn a recalled prompt into user input or an undo entry', () => {
    const { view, extensions, annotations } = host('');
    syncComposerDocument(view, 'earlier message', false, extensions);
    expect(view.state.doc.toString()).toBe('earlier message');
    expect(annotations).toEqual([true]);
    expect(undoDepth(view.state)).toBe(0);
  });
  it('cannot undo back to a previous chat when identity changes before draft clears', () => {
    const { view, extensions } = host('previous chat draft');
    view.dispatch({ changes: { from: 0, insert: 'edited ' } });
    expect(undoDepth(view.state)).toBe(1);
    syncComposerDocument(view, view.state.doc.toString(), true, extensions);
    syncComposerDocument(view, '', false, extensions);
    expect(view.state.doc.toString()).toBe('');
    expect(undoDepth(view.state)).toBe(0);
  });
});
