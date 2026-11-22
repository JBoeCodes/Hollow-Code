import { onCleanup, onMount, createEffect, createMemo, Accessor } from 'solid-js';
import { parseFrontmatter, WIKILINK_RE } from '../utils';
import { EditorView, drawSelection, highlightActiveLine, keymap, ViewPlugin, Decoration, WidgetType } from '@codemirror/view';
import { EditorState, RangeSetBuilder, Compartment, Prec, type Extension } from '@codemirror/state';
import { history, historyKeymap, defaultKeymap, indentWithTab } from '@codemirror/commands';
import { markdown, markdownLanguage } from '@codemirror/lang-markdown';
import { HighlightStyle, syntaxHighlighting, bracketMatching, indentOnInput, syntaxTree } from '@codemirror/language';
import { closeBrackets, closeBracketsKeymap, autocompletion, completionKeymap, type CompletionContext } from '@codemirror/autocomplete';
import { tags } from '@lezer/highlight';
import { noteIndex } from '../store';
import { classifyImageUrl, remoteImagePlaceholder } from '../remoteImages';
import { handleRemoteImageClick, loadRemoteImage, loadedRemoteImage } from '../remoteImageLoader';
import { inlineAi, slashComplete, type InlineAiHost } from './inlineAi';
import '../styles/codemirror.css';
import { registerRichField, type RichComposerField } from '../composerField';
import { externalComposerDoc, syncComposerDocument } from './composerDocument';

export interface MarkdownEditorProps {
  initialDoc: string;
  onChange?: (doc: string) => void;
  onWikilinkClick?: (target: string) => void;
  hasFrontmatter?: Accessor<boolean>;
  ref?: (api: EditorApi) => void;
  /** Inline AI (overnight 10-11 B5): the selection bar, suggestions and the
   *  slash menu. Absent: none of them. */
  ai?: InlineAiHost;
  /** Composer mode keeps Markdown source, source-offset selection and send keys. */
  composer?: {
    key: string | number;
    label: string;
    description: string;
    onKeyDown: (event: KeyboardEvent) => void;
    onPaste: (event: ClipboardEvent) => void;
    onSelection: (caret: number) => void;
    onReady: (field: RichComposerField) => void;
    onDispose: () => void;
    onIdentityChange: () => void;
  };
}

export interface EditorApi {
  /** Jump to a one-based source line from the note outline. */
  focusLine: (line: number) => void;
  /** Move the cursor into the frontmatter region (line 2). */
  focusFrontmatter: () => void;
  /** Replace a doc range without touching the cursor / scroll position
   *  beyond CodeMirror's automatic adjustment. */
  replaceRange: (from: number, to: number, content: string) => void;
}

const HIDE_NODES = new Set(['HeaderMark', 'EmphasisMark', 'CodeMark', 'LinkMark', 'URL', 'StrikethroughMark']);

// A zero-content replacement used to collapse markup off the active line.
const HIDE = Decoration.replace({});

// Inline markup (==, %%, [^…], #tag) must not fire inside code spans /
// fenced code — guard each match by resolving its syntax-tree context.
function inCode(state: EditorState, pos: number): boolean {
  for (let n: any = syntaxTree(state).resolveInner(pos, 1); n; n = n.parent) {
    const nm = n.name;
    if (nm === 'InlineCode' || nm === 'FencedCode' || nm === 'CodeText' || nm === 'CodeBlock') return true;
  }
  return false;
}

const cmTheme = EditorView.theme({
  '&': { backgroundColor: 'transparent', color: 'var(--ink)', fontSize: 'var(--fs-prose)', height: 'auto' },
  '&.cm-focused': { outline: 'none' },
  '.cm-content': {
    fontFamily: 'var(--ui)',
    padding: '0', caretColor: 'var(--primary)', minHeight: 'calc(100vh - 240px)',
  },
  '.cm-line': { padding: '0', lineHeight: '1.7' },
  '.cm-cursor, .cm-dropCursor': { borderLeftColor: 'var(--primary)' },
  '.cm-selectionBackground, ::selection, .cm-content ::selection': {
    backgroundColor: 'var(--primary-soft) !important',
  },
  '.cm-gutters': { display: 'none' },
  '.cm-activeLine': { backgroundColor: 'transparent' },
  '.cm-activeLineGutter': { backgroundColor: 'transparent' },
});

const cmHighlight = HighlightStyle.define([
  { tag: tags.heading1, fontFamily: 'var(--display)', fontSize: 'var(--fs-display)', fontWeight: '600', color: 'var(--ink)', letterSpacing: 'var(--ls-tight)', lineHeight: '1.2' },
  { tag: tags.heading2, fontFamily: 'var(--display)', fontSize: 'var(--fs-xl)', fontWeight: '600', color: 'var(--ink)', lineHeight: '1.25' },
  { tag: tags.heading3, fontFamily: 'var(--display)', fontSize: 'var(--fs-lg)', fontWeight: '600', color: 'var(--ink)' },
  { tag: tags.heading4, fontWeight: '600', color: 'var(--ink)' },
  { tag: tags.heading5, fontWeight: '600', color: 'var(--ink)' },
  { tag: tags.heading6, fontWeight: '600', color: 'var(--ink)' },
  { tag: tags.strong, fontWeight: '600', color: 'var(--ink)' },
  { tag: tags.emphasis, fontStyle: 'italic' },
  { tag: tags.strikethrough, textDecoration: 'line-through', color: 'var(--muted)' },
  { tag: tags.link, color: 'var(--primary)' },
  { tag: tags.url, color: 'var(--muted)' },
  { tag: tags.monospace, fontFamily: 'var(--mono)', color: 'var(--ink)', fontSize: 'var(--fs-code)' },
  { tag: tags.quote, fontStyle: 'italic', color: 'var(--muted)' },
  { tag: tags.processingInstruction, color: 'var(--muted-soft)' },
  { tag: tags.contentSeparator, color: 'var(--muted-soft)' },
]);

// Hide markdown markers off the active line.
const liveHideMarkers = ViewPlugin.fromClass(class {
  decorations: any;
  constructor(view: EditorView) { this.decorations = this.compute(view); }
  update(u: any) {
    if (u.docChanged || u.selectionSet || u.viewportChanged || u.focusChanged) {
      this.decorations = this.compute(u.view);
    }
  }
  compute(view: EditorView) {
    const builder = new RangeSetBuilder<Decoration>();
    const sel = view.state.selection.main;
    // Reveal markers only on the focused active line; when the editor is
    // unfocused the whole note renders as a clean preview (no active line).
    const activeLine = view.hasFocus ? view.state.doc.lineAt(sel.head).number : -1;
    const ranges: [number, number][] = [];
    for (const { from, to } of view.visibleRanges) {
      syntaxTree(view.state).iterate({
        from, to,
        enter: (node) => {
          if (!HIDE_NODES.has(node.type.name)) return;
          // Images render as <img> widgets (imageDecoPlugin); leave their
          // LinkMark/URL alone so the two plugins don't both replace them.
          if ((node.name === 'LinkMark' || node.name === 'URL') && node.matchContext(['Image'])) return;
          const ln = view.state.doc.lineAt(node.from).number;
          if (ln === activeLine) return;
          ranges.push([node.from, node.to]);
        },
      });
    }
    ranges.sort((a, b) => a[0] - b[0] || a[1] - b[1]);
    let lastTo = -1;
    for (const [from, to] of ranges) {
      if (from < lastTo) continue;
      builder.add(from, to, Decoration.replace({}));
      lastTo = to;
    }
    return builder.finish();
  }
}, { decorations: (v: any) => v.decorations });

// Wrap selection / skip past closing markers for *, _, `, ~
const mdAutoPair = EditorView.inputHandler.of((view, from, to, text) => {
  if (!['*', '_', '`', '~'].includes(text)) return false;
  const sel = view.state.selection.main;
  if (!sel.empty) {
    const wrapped = view.state.sliceDoc(sel.from, sel.to);
    view.dispatch({
      changes: { from: sel.from, to: sel.to, insert: text + wrapped + text },
      selection: { anchor: sel.from + 1, head: sel.to + 1 },
      userEvent: 'input.type.wrap',
    });
    return true;
  }
  if (view.state.sliceDoc(to, to + 1) === text) {
    view.dispatch({ selection: { anchor: to + 1 }, userEvent: 'input.type.skip' });
    return true;
  }
  return false;
});

export type FrontmatterRange = { end: number; endLine: number };

// Locate leading YAML frontmatter (`---\n...\n---\n`) line-by-line so cursor
// moves and edits never allocate `doc.toString()` over large notes.
export function frontmatterRange(doc: EditorState['doc']): FrontmatterRange | null {
  if (doc.lines < 3 || doc.line(1).text !== '---') return null;
  for (let i = 3; i <= doc.lines; i++) {
    const line = doc.line(i);
    if (/^---\s*$/.test(line.text)) {
      const end = i < doc.lines ? doc.line(i + 1).from : line.to;
      return { end, endLine: i };
    }
  }
  return null;
}

// Hide the YAML frontmatter block (`---\n...\n---\n`) at the start of
// the doc by adding a `display:none` class to every line in the FM
// range whenever the cursor/selection sits outside the block. Moving
// the selection into the frontmatter range (e.g. via `focusFrontmatter`)
// un-hides it so the user can edit raw YAML.
//
// Line-level decorations are the most reliable way to collapse a
// multi-line region in CodeMirror 6 — they attach a CSS class to each
// `.cm-line` element so the browser handles the layout collapse, which
// avoids the layout quirks of `Decoration.replace` over multi-line ranges.
const hideFrontmatter = ViewPlugin.fromClass(class {
  decorations: any;
  range: { end: number; endLine: number } | null;
  hidden = false;
  constructor(view: EditorView) {
    this.range = frontmatterRange(view.state.doc);
    this.decorations = this.build(view);
  }
  update(u: any) {
    if (u.docChanged) {
      this.range = frontmatterRange(u.view.state.doc);
      this.decorations = this.build(u.view);
      return;
    }
    if (u.selectionSet) {
      const shouldHide = this.shouldHide(u.view);
      if (shouldHide !== this.hidden) this.decorations = this.build(u.view);
    }
  }
  shouldHide(view: EditorView): boolean {
    if (!this.range) return false;
    const end = this.range.end;
    return !view.state.selection.ranges.some(r => r.from < end);
  }
  build(view: EditorView) {
    this.hidden = this.shouldHide(view);
    if (!this.hidden || !this.range) return Decoration.none;
    const builder = new RangeSetBuilder<Decoration>();
    const doc = view.state.doc;
    for (let i = 1; i <= this.range.endLine; i++) {
      const line = doc.line(i);
      // Line decorations attach to line.from. Builder needs ranges in
      // ascending order, which the for-loop guarantees.
      builder.add(line.from, line.from, Decoration.line({ class: 'cm-hide-line' }));
    }
    return builder.finish();
  }
}, { decorations: (v: any) => v.decorations });

// ── Table rendering ─────────────────────────────────────────────────
//
// CodeMirror's markdown parser already groups GFM tables into a `Table`
// node. We walk those, then attach line-level CSS classes that paint:
//   - header row (first line) bold + accent background
//   - delimiter row (`|---|`) muted, semi-collapsed
//   - body rows zebra-striped
// The `|` pipe characters are parsed as `TableDelimiter` nodes; we mark
// those individually so column separators have a distinct color.
const tableDecoPlugin = ViewPlugin.fromClass(class {
  decorations: any;
  constructor(view: EditorView) { this.decorations = this.compute(view); }
  update(u: any) {
    if (u.docChanged || u.viewportChanged) this.decorations = this.compute(u.view);
  }
  compute(view: EditorView) {
    const doc = view.state.doc;
    const lineDecos: { from: number; deco: Decoration }[] = [];
    const markDecos: { from: number; to: number; deco: Decoration }[] = [];
    for (const { from, to } of view.visibleRanges) {
      syntaxTree(view.state).iterate({
        from, to,
        enter: (node) => {
          if (node.type.name === 'Table') {
            const startLine = doc.lineAt(node.from).number;
            const endLine = doc.lineAt(Math.max(node.from, node.to - 1)).number;
            for (let i = startLine; i <= endLine; i++) {
              const line = doc.line(i);
              const role =
                i === startLine ? 'header' :
                i === startLine + 1 ? 'delim' :
                ((i - startLine) % 2 === 0 ? 'odd' : 'even');
              lineDecos.push({
                from: line.from,
                deco: Decoration.line({ class: `cm-md-table cm-md-table-${role}` }),
              });
            }
          } else if (node.type.name === 'TableDelimiter') {
            markDecos.push({
              from: node.from, to: node.to,
              deco: Decoration.mark({ class: 'cm-md-table-pipe' }),
            });
          }
        },
      });
    }
    // RangeSetBuilder requires sorted-by-from input; iterate visits in
    // source order so lineDecos already is, but markDecos can interleave
    // with line decorations at the same line.from offset.
    const all: { from: number; to: number; deco: Decoration; isLine: boolean }[] = [];
    for (const d of lineDecos) all.push({ from: d.from, to: d.from, deco: d.deco, isLine: true });
    for (const d of markDecos) all.push({ from: d.from, to: d.to, deco: d.deco, isLine: false });
    // Line decorations must come before mark decorations at the same `from`.
    all.sort((a, b) => a.from - b.from || (a.isLine === b.isLine ? 0 : a.isLine ? -1 : 1) || a.to - b.to);
    const builder = new RangeSetBuilder<Decoration>();
    for (const d of all) builder.add(d.from, d.to, d.deco);
    return builder.finish();
  }
}, { decorations: (v: any) => v.decorations });

// ── Callouts (Obsidian-style `> [!type]` blockquotes) ───────────────
//
// First line of a blockquote that matches `^>\s*\[!(type)\]` opens a
// callout block; every contiguous `>`-prefixed line is part of it. We
// attach `cm-md-callout-<type>` line classes so CSS can render a border-
// left + tinted background per callout flavour (note, warning, info,
// tip, danger, example, quote, …). The first line also gets
// `cm-md-callout-head` so its label can be styled distinctly.
const CALLOUT_RE = /^>\s*\[!([\w-]+)\]\s*/i;
const calloutDecoPlugin = ViewPlugin.fromClass(class {
  decorations: any;
  constructor(view: EditorView) { this.decorations = this.compute(view); }
  update(u: any) {
    if (u.docChanged || u.viewportChanged) this.decorations = this.compute(u.view);
  }
  compute(view: EditorView) {
    const doc = view.state.doc;
    const items: { from: number; deco: Decoration }[] = [];
    for (const { from, to } of view.visibleRanges) {
      syntaxTree(view.state).iterate({
        from, to,
        enter: (node) => {
          if (node.type.name !== 'Blockquote') return;
          const firstLine = doc.lineAt(node.from);
          const m = firstLine.text.match(CALLOUT_RE);
          if (!m) return;
          const type = m[1].toLowerCase();
          const startLineNo = firstLine.number;
          const endLineNo = doc.lineAt(Math.max(node.from, node.to - 1)).number;
          for (let i = startLineNo; i <= endLineNo; i++) {
            const line = doc.line(i);
            const isFirst = i === startLineNo;
            const isLast = i === endLineNo;
            const cls = [
              'cm-md-callout',
              `cm-md-callout-${type}`,
              isFirst ? 'cm-md-callout-head' : '',
              isLast ? 'cm-md-callout-tail' : '',
            ].filter(Boolean).join(' ');
            items.push({ from: line.from, deco: Decoration.line({ class: cls }) });
          }
        },
      });
    }
    items.sort((a, b) => a.from - b.from);
    const builder = new RangeSetBuilder<Decoration>();
    for (const d of items) builder.add(d.from, d.from, d.deco);
    return builder.finish();
  }
}, { decorations: (v: any) => v.decorations });

// Wikilink decorations + autocomplete.
function wikilinkDecoPlugin(known: () => Set<string>) {
  return ViewPlugin.fromClass(class {
    decorations: any;
    constructor(view: EditorView) { this.decorations = this.compute(view); }
    update(u: any) {
      if (u.docChanged || u.selectionSet || u.viewportChanged || u.focusChanged) {
        this.decorations = this.compute(u.view);
      }
    }
    compute(view: EditorView) {
      const decos: { from: number; to: number; deco: Decoration }[] = [];
      const cursorLine = view.hasFocus ? view.state.doc.lineAt(view.state.selection.main.head).number : -1;
      const set = known();
      for (const { from, to } of view.visibleRanges) {
        const text = view.state.doc.sliceString(from, to);
        WIKILINK_RE.lastIndex = 0;
        let m;
        while ((m = WIKILINK_RE.exec(text))) {
          const target = m[1].trim();
          const alias = m[2]?.trim();
          const start = from + m.index;
          const end = start + m[0].length;
          const innerStart = start + 2;          // just past `[[`
          const innerEnd = end - 2;              // just before `]]`
          const onLine = view.state.doc.lineAt(start).number === cursorLine;
          const resolved = set.has(target.toLowerCase());
          // `![[…]]` is an embed/transclusion — render it as an embed chip
          // (the leading `!` hides off the active line with the brackets).
          // Full content transclusion is out of scope; the chip links through.
          const isEmbed = start > 0 && view.state.doc.sliceString(start - 1, start) === '!';
          decos.push({
            from: innerStart, to: innerEnd,
            deco: Decoration.mark({
              class: `cm-wikilink ${isEmbed ? 'cm-md-embed' : resolved ? '' : 'unresolved'}`,
              attributes: { 'data-wikilink': target },
            }),
          });
          if (!onLine) {
            // Hide the surrounding `[[` `]]` brackets (and the `!` for embeds).
            decos.push({ from: isEmbed ? start - 1 : start, to: innerStart, deco: Decoration.replace({}) });
            decos.push({ from: innerEnd, to: end, deco: Decoration.replace({}) });
            // For pipe-aliased links, also hide the `target|` prefix so
            // only the friendly alias shows. The `|` lives right after
            // the target slice inside the inner range; locate it by
            // searching for the pipe within m[0].
            if (alias !== undefined) {
              const pipeRel = m[0].indexOf('|');
              if (pipeRel > 0) {
                const pipePos = start + pipeRel + 1; // position just after `|`
                if (pipePos > innerStart && pipePos <= innerEnd) {
                  decos.push({ from: innerStart, to: pipePos, deco: Decoration.replace({}) });
                }
              }
            }
          }
        }
      }
      decos.sort((a, b) => a.from - b.from || a.to - b.to);
      const builder = new RangeSetBuilder<Decoration>();
      for (const d of decos) builder.add(d.from, d.to, d.deco);
      return builder.finish();
    }
  }, { decorations: (v: any) => v.decorations });
}

// ── Inline Obsidian markup: highlight, comments, footnotes, tags ─────
//
// Pure-regex marks (plus delimiter-hiding off the active line) for the
// inline syntax the Lezer markdown parser doesn't model: ==highlight==,
// %%comments%%, [^footnotes] / ^[inline], and #tags. Each match is
// skipped inside code so `#include` / `==x==` in a code span keep their
// literal form.
const HIGHLIGHT_RE = /==([^=\n]+?)==/g;
const COMMENT_RE = /%%([^\n]+?)%%/g;
const FOOTREF_RE = /\[\^([^\]\s]+)\]/g;
const INLINE_FOOT_RE = /\^\[([^\]\n]+)\]/g;
const TAG_RE = /(^|[\s(])#([A-Za-z][\w/-]*)/g;

const inlineMarkPlugin = ViewPlugin.fromClass(class {
  decorations: any;
  constructor(view: EditorView) { this.decorations = this.compute(view); }
  update(u: any) {
    if (u.docChanged || u.selectionSet || u.viewportChanged || u.focusChanged) this.decorations = this.compute(u.view);
  }
  compute(view: EditorView) {
    const doc = view.state.doc;
    const activeLine = view.hasFocus ? doc.lineAt(view.state.selection.main.head).number : -1;
    const out: { from: number; to: number; deco: Decoration }[] = [];
    const push = (from: number, to: number, deco: Decoration) => out.push({ from, to, deco });
    for (const { from, to } of view.visibleRanges) {
      const text = doc.sliceString(from, to);
      let m: RegExpExecArray | null;

      HIGHLIGHT_RE.lastIndex = 0;
      while ((m = HIGHLIGHT_RE.exec(text))) {
        const s = from + m.index, e = s + m[0].length;
        if (inCode(view.state, s)) continue;
        push(s + 2, e - 2, Decoration.mark({ class: 'cm-md-highlight' }));
        if (doc.lineAt(s).number !== activeLine) { push(s, s + 2, HIDE); push(e - 2, e, HIDE); }
      }

      COMMENT_RE.lastIndex = 0;
      while ((m = COMMENT_RE.exec(text))) {
        const s = from + m.index, e = s + m[0].length;
        if (inCode(view.state, s)) continue;
        if (doc.lineAt(s).number === activeLine) {
          push(s, e, Decoration.mark({ class: 'cm-md-comment' }));
        } else {
          push(s + 2, e - 2, Decoration.mark({ class: 'cm-md-comment' }));
          push(s, s + 2, HIDE); push(e - 2, e, HIDE);
        }
      }

      INLINE_FOOT_RE.lastIndex = 0;
      while ((m = INLINE_FOOT_RE.exec(text))) {
        const s = from + m.index, e = s + m[0].length;
        if (inCode(view.state, s)) continue;
        push(s, e, Decoration.mark({ class: 'cm-md-footnote' }));
      }

      FOOTREF_RE.lastIndex = 0;
      while ((m = FOOTREF_RE.exec(text))) {
        const s = from + m.index, e = s + m[0].length;
        if (inCode(view.state, s)) continue;
        const line = doc.lineAt(s);
        const isDef = line.from === s && doc.sliceString(e, e + 1) === ':';
        push(s, e, Decoration.mark({ class: isDef ? 'cm-md-footnote-def' : 'cm-md-footnote' }));
      }

      TAG_RE.lastIndex = 0;
      while ((m = TAG_RE.exec(text))) {
        const lead = m[1] ? m[1].length : 0;
        const s = from + m.index + lead, e = s + 1 + m[2].length;
        if (inCode(view.state, s)) continue;
        push(s, e, Decoration.mark({ class: 'cm-md-tag', attributes: { 'data-tag': m[2] } }));
      }
    }
    return Decoration.set(out.map(d => d.deco.range(d.from, d.to)), true);
  }
}, { decorations: (v: any) => v.decorations });

// ── Task-list checkboxes ─────────────────────────────────────────────
//
// GFM TaskMarker nodes (`[ ]` / `[x]`) become real, clickable checkboxes;
// toggling one rewrites the marker in the document so the change persists.
class CheckboxWidget extends WidgetType {
  constructor(readonly checked: boolean, readonly from: number, readonly to: number) { super(); }
  eq(o: CheckboxWidget) { return o.checked === this.checked && o.from === this.from; }
  toDOM(view: EditorView) {
    const box = document.createElement('input');
    box.type = 'checkbox';
    box.checked = this.checked;
    box.className = 'cm-md-task-checkbox';
    box.addEventListener('mousedown', e => e.preventDefault());
    box.addEventListener('change', () => {
      view.dispatch({
        changes: { from: this.from, to: this.to, insert: box.checked ? '[x]' : '[ ]' },
        userEvent: 'input.task.toggle',
      });
    });
    return box;
  }
  ignoreEvent() { return true; }
}

const taskListPlugin = ViewPlugin.fromClass(class {
  decorations: any;
  constructor(view: EditorView) { this.decorations = this.compute(view); }
  update(u: any) {
    if (u.docChanged || u.viewportChanged || u.selectionSet) this.decorations = this.compute(u.view);
  }
  compute(view: EditorView) {
    const doc = view.state.doc;
    const out: { from: number; to: number; deco: Decoration }[] = [];
    for (const { from, to } of view.visibleRanges) {
      syntaxTree(view.state).iterate({
        from, to,
        enter: (node) => {
          if (node.name !== 'TaskMarker') return;
          const checked = /\[[xX]\]/.test(doc.sliceString(node.from, node.to));
          out.push({ from: node.from, to: node.to, deco: Decoration.replace({ widget: new CheckboxWidget(checked, node.from, node.to) }) });
          if (checked) {
            const line = doc.lineAt(node.from);
            out.push({ from: line.from, to: line.from, deco: Decoration.line({ class: 'cm-md-task-done' }) });
          }
        },
      });
    }
    return Decoration.set(out.map(d => d.deco.range(d.from, d.to)), true);
  }
}, { decorations: (v: any) => v.decorations });

// ── Inline images ────────────────────────────────────────────────────
//
// `![alt](…)` renders as an image off the active line; on the active line
// the raw source shows so it stays editable. data:/blob: images render
// directly. A remote (http/https) image is a click-to-load placeholder that
// names its host (merged plan slice 1, review A·S3): opening a note must not
// fire a request the note's author chose. The click fetches the bytes through
// the backend (the CSP allows no remote images). Local-file images stay as
// source (the WebView CSP only allows data/blob).
class ImageWidget extends WidgetType {
  constructor(readonly url: string, readonly alt: string) { super(); }
  eq(o: ImageWidget) { return o.url === this.url && o.alt === this.alt; }
  toDOM() {
    const wrap = document.createElement('span');
    wrap.className = 'cm-md-image';
    wrap.appendChild(imageEl(this.url, this.alt));
    return wrap;
  }
}

function imageEl(src: string, alt: string) {
  const img = document.createElement('img');
  img.src = src;
  img.alt = alt;
  img.loading = 'lazy';
  return img;
}

class RemoteImageWidget extends WidgetType {
  constructor(readonly url: string, readonly host: string, readonly alt: string) { super(); }
  eq(o: RemoteImageWidget) { return o.url === this.url && o.alt === this.alt; }
  toDOM() {
    const wrap = document.createElement('span');
    wrap.className = 'cm-md-image';
    const hit = loadedRemoteImage(this.url);
    if (hit) { wrap.appendChild(imageEl(hit, this.alt)); return wrap; }
    const tmp = document.createElement('span');
    tmp.innerHTML = remoteImagePlaceholder(this.url, this.host, this.alt);
    const btn = tmp.firstElementChild as HTMLButtonElement;
    btn.addEventListener('mousedown', e => e.preventDefault());
    btn.addEventListener('click', e => {
      if (!handleRemoteImageClick(e)) return;
      void loadRemoteImage(this.url).then(data => { btn.replaceWith(imageEl(data, this.alt)); }, () => {});
    });
    wrap.appendChild(btn);
    return wrap;
  }
}

const IMG_INLINE = /^(data:|blob:)/i;
const imageDecoPlugin = ViewPlugin.fromClass(class {
  decorations: any;
  constructor(view: EditorView) { this.decorations = this.compute(view); }
  update(u: any) {
    if (u.docChanged || u.viewportChanged || u.selectionSet || u.focusChanged) this.decorations = this.compute(u.view);
  }
  compute(view: EditorView) {
    const doc = view.state.doc;
    const activeLine = view.hasFocus ? doc.lineAt(view.state.selection.main.head).number : -1;
    const out: { from: number; to: number; deco: Decoration }[] = [];
    for (const { from, to } of view.visibleRanges) {
      syntaxTree(view.state).iterate({
        from, to,
        enter: (node) => {
          if (node.name !== 'Image') return;
          if (doc.lineAt(node.from).number === activeLine) return;
          const mm = doc.sliceString(node.from, node.to).match(/^!\[([^\]]*)\]\(\s*([^)\s]+)(?:\s+"[^"]*")?\s*\)$/);
          if (!mm) return;
          const kind = classifyImageUrl(mm[2]);
          const widget = IMG_INLINE.test(mm[2]) ? new ImageWidget(mm[2], mm[1])
            : kind.kind === 'remote' ? new RemoteImageWidget(kind.url, kind.host, mm[1])
            : null;
          if (!widget) return;
          out.push({ from: node.from, to: node.to, deco: Decoration.replace({ widget }) });
        },
      });
    }
    return Decoration.set(out.map(d => d.deco.range(d.from, d.to)), true);
  }
}, { decorations: (v: any) => v.decorations });

// ── Horizontal rules ─────────────────────────────────────────────────
//
// `---` / `***` / `___` render as a drawn rule off the active line (CSS
// collapses the source text and paints a border); the literal markers
// reappear when the cursor sits on the line.
const hrDecoPlugin = ViewPlugin.fromClass(class {
  decorations: any;
  constructor(view: EditorView) { this.decorations = this.compute(view); }
  update(u: any) {
    if (u.docChanged || u.viewportChanged || u.selectionSet || u.focusChanged) this.decorations = this.compute(u.view);
  }
  compute(view: EditorView) {
    const doc = view.state.doc;
    const activeLine = view.hasFocus ? doc.lineAt(view.state.selection.main.head).number : -1;
    const builder = new RangeSetBuilder<Decoration>();
    for (const { from, to } of view.visibleRanges) {
      syntaxTree(view.state).iterate({
        from, to,
        enter: (node) => {
          if (node.name !== 'HorizontalRule') return;
          const line = doc.lineAt(node.from);
          if (line.number === activeLine) return;
          builder.add(line.from, line.from, Decoration.line({ class: 'cm-md-hr' }));
        },
      });
    }
    return builder.finish();
  }
}, { decorations: (v: any) => v.decorations });

// ── Unordered list bullets ───────────────────────────────────────────
//
// `-` / `*` / `+` list markers render as a • bullet (Obsidian keeps the
// bullet visible at all times, including on the active line). Task items
// hide the marker entirely — their checkbox is the marker.
class BulletWidget extends WidgetType {
  eq() { return true; }
  toDOM() {
    const span = document.createElement('span');
    span.className = 'cm-md-bullet';
    span.textContent = '•';
    return span;
  }
}
const BULLET = Decoration.replace({ widget: new BulletWidget() });

const bulletDecoPlugin = ViewPlugin.fromClass(class {
  decorations: any;
  constructor(view: EditorView) { this.decorations = this.compute(view); }
  update(u: any) {
    if (u.docChanged || u.viewportChanged) this.decorations = this.compute(u.view);
  }
  compute(view: EditorView) {
    const doc = view.state.doc;
    const builder = new RangeSetBuilder<Decoration>();
    for (const { from, to } of view.visibleRanges) {
      syntaxTree(view.state).iterate({
        from, to,
        enter: (node) => {
          if (node.name !== 'ListMark') return;
          const mark = doc.sliceString(node.from, node.to);
          if (mark !== '-' && mark !== '*' && mark !== '+') return; // ordered marks stay numeric
          const line = doc.lineAt(node.from);
          const rest = doc.sliceString(node.to, line.to);
          // Task item (`- [ ]`): hide the marker, the checkbox stands in.
          builder.add(node.from, node.to, /^\s*\[[ xX]\]/.test(rest) ? HIDE : BULLET);
        },
      });
    }
    return builder.finish();
  }
}, { decorations: (v: any) => v.decorations });

function wikilinkComplete(getNotes: () => { path: string; basename: string }[]) {
  return (context: CompletionContext) => {
    const before = context.matchBefore(/\[\[([^\]\[\n]*)$/);
    if (!before || (before.from === before.to && !context.explicit)) return null;
    const query = before.text.slice(2).toLowerCase();
    const options = getNotes()
      .filter(n => n.basename.toLowerCase().includes(query))
      .slice(0, 30)
      .map(n => ({
        label: n.basename,
        apply: `${n.basename}]]`,
        detail: n.path.split('/').slice(0, -1).join('/') || 'root',
      }));
    return { from: before.from + 2, options, validFor: /^[^\]\[\n]*$/ };
  };
}

export function MarkdownEditor(props: MarkdownEditorProps) {
  let host!: HTMLDivElement;
  let view: EditorView | null = null;
  let unregister: (() => void) | undefined;
  let docKey = props.composer?.key;
  let extensions: Extension[] = [];

  const knownSet = createMemo(() => new Set(noteIndex().map(n => n.basename.toLowerCase())));

  onMount(() => {
    const wikiClick = EditorView.domEventHandlers({
      click(e) {
        const el = (e.target as HTMLElement).closest('.cm-wikilink');
        if (!el) return false;
        e.preventDefault(); e.stopPropagation();
        const name = el.getAttribute('data-wikilink');
        if (name && props.onWikilinkClick) props.onWikilinkClick(name);
        return true;
      },
    });

    // Drop the cursor past any leading YAML frontmatter so the FM hider
    // collapses the raw block immediately. Otherwise position-0 sits inside
    // the FM range and the editor renders both the panel AND the raw `---`.
    const fmInit = props.composer ? null : parseFrontmatter(props.initialDoc);
    const initSel = fmInit ? { anchor: fmInit.bodyStart } : { anchor: 0 };

    view = new EditorView({
      parent: host,
      state: EditorState.create({
        doc: props.initialDoc,
        selection: initSel,
        extensions: extensions = [
          history(),
          drawSelection(),
          highlightActiveLine(),
          bracketMatching(),
          indentOnInput(),
          closeBrackets(),
          ...(props.composer ? [] : [autocompletion({ override: [wikilinkComplete(noteIndex), slashComplete] })]),
          ...(props.ai ? [inlineAi(props.ai)] : []),
          keymap.of([...closeBracketsKeymap, ...completionKeymap, ...defaultKeymap, ...historyKeymap, indentWithTab]),
          markdown({ base: markdownLanguage }),
          syntaxHighlighting(cmHighlight),
          liveHideMarkers,
          ...(props.composer ? [] : [hideFrontmatter]),
          tableDecoPlugin,
          calloutDecoPlugin,
          wikilinkDecoPlugin(knownSet),
          inlineMarkPlugin,
          taskListPlugin,
          imageDecoPlugin,
          hrDecoPlugin,
          bulletDecoPlugin,
          wikiClick,
          mdAutoPair,
          EditorView.lineWrapping,
          cmTheme,
          ...(props.composer ? [
            Prec.highest(EditorView.domEventHandlers({
              keydown(event, editor) {
                if (editor.composing || event.isComposing) return false;
                props.composer?.onKeyDown(event);
                return event.defaultPrevented;
              },
              paste(event) { props.composer?.onPaste(event); return event.defaultPrevented; },
            })),
            EditorView.contentAttributes.of({ role: 'textbox', 'aria-multiline': 'true', 'aria-label': props.composer.label, 'aria-describedby': props.composer.description }),
            EditorView.theme({
              '&': { fontSize: 'var(--fs-md)' },
              '.cm-content': { minHeight: '0', padding: 'var(--sp-3) var(--sp-4)' },
              '.cm-scroller': { overflow: 'auto', maxHeight: 'var(--composer-editor-cap, 40vh)' },
            }),
          ] : []),
          EditorView.updateListener.of(update => {
            if (update.docChanged && props.onChange && !update.transactions.some(t => t.annotation(externalComposerDoc))) {
              props.onChange(update.state.doc.toString());
            }
            if (update.selectionSet || update.docChanged) props.composer?.onSelection(update.state.selection.main.head);
          }),
        ],
      }),
    });
    if (props.composer) {
      const editor = view;
      const clamp = (pos: number) => Math.max(0, Math.min(editor.state.doc.length, pos));
      const field: RichComposerField = {
        element: editor.contentDOM,
        get value() { return editor.state.doc.toString(); },
        set value(text) { editor.dispatch({ changes: { from: 0, to: editor.state.doc.length, insert: text } }); },
        get selectionStart() { return editor.state.selection.main.from; },
        set selectionStart(pos) { this.setSelectionRange(pos, this.selectionEnd); },
        get selectionEnd() { return editor.state.selection.main.to; },
        set selectionEnd(pos) { this.setSelectionRange(this.selectionStart, pos); },
        get isConnected() { return editor.dom.isConnected; },
        get style() { return editor.dom.style; },
        get scrollHeight() { return editor.contentHeight; },
        get clientWidth() { return editor.contentDOM.clientWidth; },
        focus(options) { editor.contentDOM.focus(options); },
        setSelectionRange(from, to) { editor.dispatch({ selection: { anchor: clamp(from), head: clamp(to) } }); },
        getClientRects: () => editor.dom.getClientRects(),
        closest: selector => editor.dom.closest(selector),
        dispatchEvent: event => editor.contentDOM.dispatchEvent(event),
        replaceText(from, to, text) {
          editor.dispatch({ changes: { from: clamp(from), to: clamp(to), insert: text }, selection: { anchor: clamp(from) + text.length }, userEvent: 'input.type' });
        },
        caretEdges() {
          const sel = editor.state.selection.main;
          if (!sel.empty) return { atTop: false, atBottom: false };
          const here = editor.coordsAtPos(sel.head), first = editor.coordsAtPos(0), last = editor.coordsAtPos(editor.state.doc.length);
          return { atTop: !!here && !!first && Math.abs(here.top - first.top) < 2, atBottom: !!here && !!last && Math.abs(here.bottom - last.bottom) < 2 };
        },
      };
      unregister = registerRichField(field);
      props.composer.onReady(field);
    }
    // Note: the editor intentionally does NOT grab focus on open. An
    // unfocused editor has no "active line", so the whole note renders as a
    // clean preview (including the first heading) until the user clicks in.

    // Expose an imperative API for the parent (e.g. "edit raw" on the
    // Properties panel jumps the cursor into the frontmatter block, which
    // un-hides it via the hideFrontmatter plugin).
    if (props.ref) {
      props.ref({
        focusLine: (line) => {
          if (!view) return;
          const target = view.state.doc.line(Math.max(1, Math.min(view.state.doc.lines, line)));
          view.dispatch({ selection: { anchor: target.from }, effects: EditorView.scrollIntoView(target.from, { y: 'start' }) });
          view.focus();
        },
        focusFrontmatter: () => {
          if (!view) return;
          const line2 = view.state.doc.line(Math.min(2, view.state.doc.lines));
          view.dispatch({
            selection: { anchor: line2.from },
            effects: EditorView.scrollIntoView(line2.from, { y: 'start' }),
          });
          view.focus();
        },
        replaceRange: (from, to, content) => {
          if (!view) return;
          const len = view.state.doc.length;
          const safeFrom = Math.max(0, Math.min(len, from));
          const safeTo = Math.max(safeFrom, Math.min(len, to));
          view.dispatch({
            changes: { from: safeFrom, to: safeTo, insert: content },
            // Let CodeMirror map the cursor automatically.
            userEvent: 'input.replace.frontmatter',
          });
        },
      });
    }
  });

  onCleanup(() => { unregister?.(); if (props.composer) props.composer.onDispose(); view?.destroy(); view = null; });

  // Reset doc when initialDoc changes (e.g., switching notes)
  createEffect((prevDoc?: string) => {
    const doc = props.initialDoc;
    const key = props.composer?.key;
    if (view && props.composer) {
      const reset = key !== docKey;
      if (reset) props.composer.onIdentityChange();
      docKey = key;
      syncComposerDocument(view, doc, reset, extensions);
      return doc;
    }
    if (view && doc !== prevDoc && doc !== view.state.doc.toString()) {
      const fm = props.composer ? null : parseFrontmatter(doc);
      view.dispatch({
        changes: { from: 0, to: view.state.doc.length, insert: doc },
        selection: { anchor: props.composer ? Math.min(view.state.selection.main.head, doc.length) : fm ? fm.bodyStart : 0 },
      });
      // Open switched notes as a clean preview — drop focus so no line is
      // "active" until the user clicks in to edit (e.g. after a wikilink jump).
      if (!props.composer) view.contentDOM.blur();
    }
    return doc;
  });

  return <div ref={host} class={props.composer ? 'composer-markdown-host' : 'vault-editor-host'} />;
}
