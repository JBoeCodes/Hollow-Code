## Designing a canvas

Every canvas is dithered as a whole by default. Never set root `texture="plain"`. A single graphic may use `plain`; `ascii` is the alternative. Keep text and controls crisp on solid plates. Add small content-specific dithered art using the dithered-design skill. Host accessibility and Texture Off always win.

The kit owns type, spacing, colour and dark mode. Choose and order:
- The title states the conclusion ("Release slips to Friday"), not a topic.
- Lead with 2–4 `hc-stat`s. Values under 10 characters; qualifiers in `delta`.
  Tones carry meaning; one warning stands out.
- One evidence block: a value as a stat, bounded values as a gauge,
  categories as bars, time as a line, shares as a donut, else a table
  with at most 8 rows. Never repeat the same numbers.
- End with what happens next: `hc-steps` with the current step, or one
  `hc-callout` naming the decision.
- `hc-flow`: at most 12 nodes, labels of 2–5 words, groups only for real
  boundaries. Prefer `down`; `right` only for 3–5 steps.
- Use kit elements, not hand-made cards or tables, and no colours of your
  own: they break in dark mode.
- Keep headings ordered, chart data accessible through the kit, meaning
  in words and inputs labelled. Add no motion.
- A canvas button only fills a draft the person sends; never imply it acts.

A status canvas:

    <hc-canvas id="sync-status" title="Sync is healthy; one retry queue">
      <hc-stats>
        <hc-stat label="Synced today" value="1,284" delta="+12%" tone="success"></hc-stat>
        <hc-stat label="Retrying" value="3" delta="Notion rate limit" tone="warning"></hc-stat>
        <hc-stat label="Last run" value="09:40"></hc-stat>
      </hc-stats>
      <hc-chart label="Items synced per day">
        Mon: 980
        Tue: 1,140
        Wed: 1,284
      </hc-chart>
      <hc-callout tone="info" title="Next">The three retries clear on the 10:00 run.</hc-callout>
    </hc-canvas>

A decision canvas:

    <hc-canvas id="host-choice" title="Host the API on Fly">
      <hc-compare label="Where the API runs">
        <div label="Fly" recommended>Closest to users; $12 a month.</div>
        <div label="Render">Simplest deploys; $19 a month.</div>
      </hc-compare>
      <hc-flow label="Request path">
        app: App
        api: API {success}
        db: Postgres
        app -> api -> db
      </hc-flow>
    </hc-canvas>
