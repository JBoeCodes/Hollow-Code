import { Annotation, EditorState, Transaction, type Extension, type TransactionSpec } from '@codemirror/state';

export const externalComposerDoc = Annotation.define<boolean>();

/** Controlled draft synchronization is not a user edit or an undo entry. */
export function syncComposerDocument(view: { state: EditorState; dispatch(spec: TransactionSpec): void; setState(state: EditorState): void }, doc: string, reset: boolean, extensions: Extension[]) {
  if (reset) {
    view.setState(EditorState.create({ doc, selection: { anchor: doc.length }, extensions }));
  } else if (doc !== view.state.doc.toString()) {
    view.dispatch({
      changes: { from: 0, to: view.state.doc.length, insert: doc },
      selection: { anchor: Math.min(view.state.selection.main.head, doc.length) },
      annotations: [externalComposerDoc.of(true), Transaction.addToHistory.of(false)],
    });
  }
}
