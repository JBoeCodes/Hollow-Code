/**
 * PLAN.md as sections of checklist rows (Canvas C5a, plan §6). Pure, no DOM.
 *
 * Each `##` heading is a section with its own counts; rows are grouped open
 * and done; a section whose heading says it waits on the person ("Waiting on
 * the user", "Needs you") is flagged. Every row carries its exact line
 * identity, the 1-based line number plus the SHA-256 of that line's text, so
 * a later toggle can patch PLAN.md compare-and-swap: it flips the box only if
 * that line still reads exactly the same. Everything that isn't a row stays
 * as the Markdown it is (`notes`), so no text is dropped. Nothing here writes.
 */

import { headingOf, indentOf, isBlank, joinLines, listItemOf, plainText, readSections, sha256Hex, type SourceLine } from './sectionMarkdown';
import { normalizePlanText } from './taskOrigin';

/** Where a row is, exactly: the toggle refuses if the line no longer hashes the same. */
export interface PlanRowId {
  /** 1-based line number in PLAN.md. */
  line: number;
  /** Lowercase hex SHA-256 of the line's UTF-8 text, without its `\n` or `\r\n`. */
  sha256: string;
}

export interface PlanRow {
  id: PlanRowId;
  /** The item text after the box, as Markdown, continuation lines joined with a space. */
  text: string;
  done: boolean;
  /** List nesting: 0 for a top-level item. */
  depth: number;
  /** The nearest enclosing checklist row, if any. A nested row is counted once, as itself. */
  parent: { line: number; text: string } | null;
  /** The last line the row spans (continuation lines included). */
  endLine: number;
}

export interface PlanCounts { done: number; open: number; total: number }

export interface PlanSpan { line: number; endLine: number; markdown: string }

export interface PlanSection {
  /** The `##` heading, or null for rows before any `##`. */
  heading: string | null;
  line: number;
  endLine: number;
  /** The heading says the rows wait on the person. */
  waitingOnYou: boolean;
  counts: PlanCounts;
  /** Every row in file order. */
  rows: PlanRow[];
  open: PlanRow[];
  done: PlanRow[];
  /** Everything in the section that isn't a row (prose, plain bullets, code), exact. */
  notes: PlanSpan[];
}

export interface PlanModel {
  /** YAML frontmatter at the top, delimiters included, as written. */
  frontmatter: PlanSpan | null;
  title: string | null;
  /** The `# Title` line, when there is one. */
  titleLine: number | null;
  sections: PlanSection[];
  counts: PlanCounts;
  /** The first section flagged `waitingOnYou`. */
  waitingOnYou: PlanSection | null;
}

/** "Waiting on you", "Waiting on the user", "Needs you", "Blocked on you", "For you". */
export const WAITING_HEADING = /\b(waiting (on|for) (you|the user|me|the owner)|needs? you|blocked on (you|the user|me)|for you)\b/i;

const BOX = /^\[([ xX])\](?:[ \t]+(.*)|[ \t]*)$/;

/** A line's identity for the toggle (exported so the tests and the later writer agree). */
export const planRowId = (line: number, text: string): PlanRowId => ({ line, sha256: sha256Hex(text) });

const counts = (rows: PlanRow[]): PlanCounts => {
  const done = rows.filter(r => r.done).length;
  return { done, open: rows.length - done, total: rows.length };
};

export function readPlan(text: string): PlanModel {
  const doc = readSections(text);
  let title: string | null = null;
  const sections: PlanSection[] = [];
  let titleLine: number | null = null;
  for (const s of doc.sections) {
    // The first `# Title` names the plan; its body is the untitled lead section.
    const isTitle = s.level === 1 && title === null;
    if (isTitle) { title = s.heading; titleLine = s.line; }
    const sec = readSection(isTitle ? null : s.heading, s.line, s.endLine, s.body);
    if (isTitle && !sec.rows.length && !sec.notes.length) continue;
    sections.push(sec);
  }
  const all = sections.flatMap(s => s.rows);
  const fm = doc.frontmatter;
  const frontmatter = fm ? { line: fm.line, endLine: fm.endLine, markdown: joinLines(doc.lines.slice(0, fm.endLine)) } : null;
  return { frontmatter, title, titleLine, sections, counts: counts(all), waitingOnYou: sections.find(s => s.waitingOnYou) ?? null };
}

function readSection(heading: string | null, line: number, endLine: number, body: SourceLine[]): PlanSection {
  const rows: PlanRow[] = [];
  const notes: PlanSpan[] = [];
  let note: SourceLine[] = [];
  // Open list items, innermost last: their indent and the row they are (null for a plain bullet).
  const stack: { indent: number; row: PlanRow | null }[] = [];
  let last: { row: PlanRow; indent: number } | null = null;
  const flushNote = () => {
    while (note.length && isBlank(note[note.length - 1])) note.pop();
    while (note.length && isBlank(note[0])) note.shift();
    if (note.length) notes.push({ line: note[0].n, endLine: note[note.length - 1].n, markdown: joinLines(note) });
    note = [];
  };
  for (const l of body) {
    const li = listItemOf(l);
    if (li) {
      while (stack.length && stack[stack.length - 1].indent >= li.indent) stack.pop();
      const box = BOX.exec(li.rest);
      if (box) {
        flushNote();
        const parentRow = [...stack].reverse().find(e => e.row)?.row ?? null;
        const row: PlanRow = {
          id: planRowId(l.n, l.text),
          text: (box[2] ?? '').trim(),
          done: box[1] !== ' ',
          depth: stack.length,
          parent: parentRow ? { line: parentRow.id.line, text: parentRow.text } : null,
          endLine: l.n,
        };
        rows.push(row);
        stack.push({ indent: li.indent, row });
        last = { row, indent: li.indent };
      } else {
        stack.push({ indent: li.indent, row: null });
        last = null;
        note.push(l);
      }
      continue;
    }
    if (isBlank(l)) { last = null; note.push(l); continue; }
    if (last && !l.fenced && !headingOf(l) && indentOf(l) > last.indent) {
      // An indented continuation of the row above.
      last.row.text = `${last.row.text} ${l.text.trim()}`.trim();
      last.row.endLine = l.n;
      continue;
    }
    if (!/^\s/.test(l.text)) stack.length = 0;
    last = null;
    note.push(l);
  }
  flushNote();
  const done = rows.filter(r => r.done);
  const open = rows.filter(r => !r.done);
  return {
    heading, line, endLine, waitingOnYou: heading !== null && WAITING_HEADING.test(heading),
    counts: counts(rows), rows, open, done, notes,
  };
}

/** PLAN.md's checklist rows in file order: the nth checkbox the Markdown
 *  view draws is the nth row, when the counts agree (else nothing ticks;
 *  COO rebuild L4, the Files tab's PLAN.md). */
export function planRowsInOrder(text: string): PlanRow[] {
  return readPlan(text).sections.flatMap(s => s.rows).sort((a, b) => a.id.line - b.id.line);
}

/** The line of the row a task was copied from (its text, compared as
 *  `normalizePlanText` does), for the Tasks "From: plan" link, which opens
 *  PLAN.md at that row; null when no row has it now. */
export function planItemLine(text: string, item: string): number | null {
  const want = normalizePlanText(plainText(item));
  return planRowsInOrder(text).find(r => normalizePlanText(plainText(r.text)) === want)?.id.line ?? null;
}
