## Canvas Kit (kit@1)

Prefer themed elements to custom CSS/JS. Attributes: plain text. Tones: `info`, `success`, `warning`, `danger` (else neutral); each adds an icon/word.

- `<hc-stats>` holding `<hc-stat label value delta tone>` tiles: numbers.
- `<hc-progress label value max tone>`: a bar (`max` defaults to 100).
- `<hc-steps label current>`: one child per step with `label` and optional
  `when` and `status` (`done`, `current`, `upcoming`, `blocked`); the child's
  text is its detail; `current` counts from 1.
- `<hc-checklist label>`: static rows `[x] Done` / `[ ] Not done`. In `hc-ask`,
  `name` yields {row: Pass/Fail/Skip}; `required` means every row.
- `<hc-bars label unit max>`: horizontal bars for one series, from data rows.
- `<hc-chart type label series unit height>`: `type` is `line` (default),
  `bar`, `area` or `donut`; data rows or a `<table>`; it comes with a table
  view and keyboard values. Charts enter once; Reduce motion skips motion.
- `<hc-gauge label value min max unit tone>`: semicircle, exact value; min/max default to 0/100. Missing value means no data.
- `<hc-sparkline label unit>`: compact trend from data rows; missing samples
  leave gaps. May sit inside an `hc-stat`. First series drawn, table has all.
- `<hc-timeline label>`: horizontal dated events/spans; rows are
  `2026-10-12 | Design | 2026-10-14` (end date optional). At most 60 plotted.
- `<hc-heatmap label unit start end>`: calendar grid, rows `2026-10-12: 8`;
  optional start/end are YYYY-MM-DD, at most 366 days. Missing dates are
  unknown; repeated dates use the last value. Arrows read values.
- `<hc-table label>`: a `<table>` or data rows, the first row the header;
  the person can sort it.
- `<hc-compare label>`: one child per option with `label`; mark the one you
  recommend with `recommended`.
- `<hc-tabs label>`: a child per panel with `label` (`selected` on one).
- `<hc-details summary open>`: a collapsible section.
- `<hc-callout tone title>`: a note, caveat or risk.
- `<hc-badge tone>`: a status word.
- `<hc-file path>`: names a vault file.
- `<hc-code lang label>`: code as text.
- `<hc-flow label direction interactive>`: boxes and routed arrows;
  `direction="right"` runs across. One line each: `id: Label {shape,
  tone}` (shapes `start`, `end`, `decision`, `note`), `a -> b: label` (or a
  chain `a -> b -> c`), and `[Group]` before the nodes it boxes (`[]` ends
  it). End a link with `{active}` for flow motion; hover/focus also moves.
  Big flows have pan/zoom and Fit; `interactive` forces controls.

Data rows, one per line: `Label: 12`, or `Label: 12, 8` for two series, or
`Label | 12 | 8` under a header row such as `Week | Claude | Codex`. Values
like `1,200`, `$42`, `12%` and `3 h` are numbers. Name series with
`series="A, B"` or the header row. For example:

    <hc-chart type="bar" label="Hours by lane" unit="h">
      Lane A: 12
      Lane B: 30
    </hc-chart>

Outside data rows, ask options and `hc-code`, write `&lt;` for text `<` (bare `<title>` hides the rest).

One screen: stats, chart/table, next steps.
