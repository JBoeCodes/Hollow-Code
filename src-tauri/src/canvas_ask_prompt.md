An ask-canvas is a canvas whose root carries `ask`, holding one `<hc-ask>`
with its fields. Say what you are asking in prose first, then end your turn
after the fence: don't guess the answer or carry on.

```hollow-canvas
<hc-canvas id="launch-order" title="Launch order" ask>
  <hc-ask title="Launch order" send="Use these answers">
    <hc-rank name="order" label="What goes out first?">
    Docs
    Beta
    Pricing page
    </hc-rank>
    <hc-range name="budget" label="Launch budget" min="0" max="50" step="5" value="20" unit="k$"></hc-range>
    <hc-choice name="day" label="When?" required>
    * This Friday
    Next week
    </hc-choice>
  </hc-ask>
</hc-canvas>
```

The ask kit (kit@1, only in ask-canvases). Every field takes `name` (the key
its answer arrives under), `label` (the question) and `required`. Options
are one per line; `* ` before one marks the one you recommend.
- `<hc-ask title send>`: the form; `send` names Hollow's Send button.
- `<hc-choice>`: one option. `<hc-multi min max>`: several.
- `<hc-range min max step value unit>`: a number (null until set when
  there is no `value`).
- `<hc-rank>`: every item, in the person's order.
- `<hc-text long maxlength placeholder>`: short or long text.
- `<hc-date min max value>`: a date, `YYYY-MM-DD`.
- `<hc-matrix options>`: one option per row
  (`options="Low | Medium | High"`).
- `<hc-pick unit>`: one of its data rows (`Week 1: 12`), drawn as bars.

Hollow draws the Send button under the canvas; a canvas script can't send.
When the person presses it, Hollow sends the answers straight to you. The
answer arrives as their next message: `Answered canvas "<id>":`, one
`- label: value` line per answer (cut short when long), then a
`hollow-canvas-answer` block holding `{"canvasId", "revision", "answers"}`
as JSON. Trust the JSON: `answers` maps each field's `name` to its value
(null when left empty). Once sent, the canvas locks; to ask again, emit it
again with the same `id` (a new version) or use a new one.
