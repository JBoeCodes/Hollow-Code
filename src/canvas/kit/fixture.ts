/** A kit@1 document body used by the smoke harness (`?window=smoke-artifact`)
 *  and by tests/smoke/canvas-kit.spec.ts. Synthetic data only. The inline
 *  script marks whether artifact scripts ran; the hostile attribute proves
 *  values render as text. */
export const KIT_FIXTURE = `
<main data-fixture="kit">
  <h1 style="margin:0;font-size:20px">Kit fixture</h1>
  <hc-stats label="Release">
    <hc-stat label="Bed checks" value="38" delta="0 fail" tone="success"></hc-stat>
    <hc-stat label="Skipped" value="13" delta="+2"></hc-stat>
    <hc-stat label="Injected" value="<img src=x onerror=alert(1)>"></hc-stat>
  </hc-stats>
  <hc-progress label="Plan" value="3" max="5" tone="info"></hc-progress>
  <hc-callout tone="warning" title="Heads up">Install waits for the bed.</hc-callout>
  <p>Status <hc-badge tone="danger">Blocked</hc-badge></p>
  <hc-details summary="Why it matters">Hidden reasoning body.</hc-details>
  <script>document.body.dataset.scriptRan = '1';</script>
</main>`;

/** The K2 display components (`?window=smoke-artifact`, region "Canvas kit
 *  display fixture"; tests/smoke/canvas-kit-display.spec.ts). Text rows,
 *  a table source, a chart in a hidden tab panel, and a refused web path. */
export const KIT_DISPLAY_FIXTURE = `
<main data-fixture="kit-display">
  <h1 style="margin:0;font-size:20px">Display fixture</h1>
  <hc-steps label="Release" current="2">
    <div label="Spike" when="Oct 3">Exfil matrix</div>
    <div label="Sandbox" when="Oct 4">Canvas mode</div>
    <div label="Kit"></div>
    <div label="Install" status="blocked">Waits for the bed</div>
  </hc-steps>
  <hc-bars label="Hours by lane" unit="h">
    Lane A: 12
    Lane B: 30
    Lane C: 7.5
  </hc-bars>
  <hc-chart type="line" label="Sessions per day" series="Claude, Codex">
    Mon: 12, 8
    Tue: 15, 9
    Wed: 11, 14
    Thu: 18, 12
    Fri: 21, 10
  </hc-chart>
  <hc-chart type="bar" label="Checks by suite">
    Suite | Passed | Skipped
    Rust | 2,077 | 12
    Vitest | 1,025 | 0
    Smoke | 471 | 113
  </hc-chart>
  <hc-chart type="donut" label="Spend by provider">
    <table>
      <tr><th>Provider</th><th>Spend</th></tr>
      <tr><td>Claude</td><td>$42</td></tr>
      <tr><td>Codex</td><td>$18</td></tr>
      <tr><td>Grok</td><td>$6</td></tr>
    </table>
  </hc-chart>
  <hc-table label="Lanes">
    Lane | Hours | Status
    Lane A | 12 | done
    Lane B | 30 | open
    Lane C | 7.5 | done
    Lane D | | blocked
  </hc-table>
  <hc-compare label="Hosting">
    <div label="Fly" recommended>Closer to users.</div>
    <div label="Render">Simpler deploys.</div>
  </hc-compare>
  <hc-tabs label="Views">
    <section label="Summary">First panel text.</section>
    <section label="Queue">
      <hc-chart type="area" label="Queue depth">
        09:00: 3
        10:00: 7
        11:00: 4
        12:00: 9
      </hc-chart>
    </section>
    <section label="Notes">Third panel text.</section>
  </hc-tabs>
  <p>Files: <hc-file path="outputs/report.md">Report</hc-file> <hc-file path="https://example.invalid/x">Web</hc-file></p>
  <hc-code lang="ts" label="example.ts">
    // greet the vault
    const name = "Hollow";
    return 42;
  </hc-code>
  <hc-flow label="Deploy decision">
    start: Change merged {start}
    ci: CI green? {decision}
    ship: Deploy {success}
    fix: Fix failures {danger}
    start -> ci
    ci -> ship: yes
    ci -> fix: no
    fix -> ci
  </hc-flow>
  <hc-flow label="Request path" direction="right">
    [Frontend]
    view: Chat view
    api: api.ts
    [Rust backend]
    cmd: Command
    db: Store {info}
    []
    view -> api: calls
    api -> cmd -> db: writes
  </hc-flow>
  <script>document.body.dataset.scriptRan = '1';</script>
</main>`;
