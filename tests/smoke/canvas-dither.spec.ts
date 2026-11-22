import { test, expect } from './fixtures/test';
import { readFileSync, mkdirSync, writeFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { execFileSync } from 'node:child_process';
import { scoreBox, contrastRatio, type Pixels } from '../../src/canvas/pixelContrast';
import { measuredFindings } from '../../src/canvas/designAudit';

const read = (p: string) => process.env.DITHER_PHASE === 'before'
  ? execFileSync('git', ['show', `b68776e9:${p}`], { encoding: 'utf8' }) : readFileSync(resolve(p), 'utf8');
const sample = `<h1>A clearer view of the work</h1>
<hc-stats><hc-stat label="Completed" value="38" delta="12 this week" tone="success"></hc-stat><hc-stat label="Needs attention" value="3" delta="Review next" tone="warning"></hc-stat><hc-stat label="Capacity" value="72%"></hc-stat></hc-stats>
<div class="sample-grid"><section>
<hc-callout title="Next review" tone="info">Review the three remaining items before Friday.</hc-callout>
<hc-progress label="Review" value="72"></hc-progress><hc-bars label="Hours" unit="h">Design: 12\nBuild: 30\nReview: 7</hc-bars>
<hc-compare label="Approach"><div label="Small batches" recommended>Keep each result reviewable.</div><div label="One batch">Review at the end.</div></hc-compare>
<hc-steps current="2"><div label="Design">Agreed</div><div label="Build">In progress</div><div label="Review">Next</div></hc-steps>
<hc-table label="Queue">Owner | Items | State\nAvery | 12 | Ready\nMorgan | 3 | Review\nRiley | 7 | Ready</hc-table>
<hc-tabs label="Views"><div label="Summary">The latest totals are above.</div><div label="Notes">Notes stay selectable.</div></hc-tabs>
<hc-details summary="Review notes" open>Check the source before accepting a result.</hc-details>
<p><hc-badge tone="success">Ready</hc-badge> <hc-badge tone="warning">Review</hc-badge> <hc-badge tone="danger">Blocked</hc-badge></p>
<hc-checklist label="Acceptance">[x] Sources checked\n[ ] Final review</hc-checklist>
<hc-file path="outputs/report.md">Report</hc-file><hc-code label="Example" lang="ts">const ready = true;</hc-code>
</section><section>
<hc-chart type="bar" height="170" label="Items by team">Avery: 12\nMorgan: 3\nRiley: 7</hc-chart>
<hc-chart type="area" height="170" label="Queue">Mon: 3\nTue: 7\nWed: 4\nThu: 9</hc-chart>
<hc-chart type="line" height="170" label="Completed">Mon: 12\nTue: 15\nWed: 11\nThu: 18</hc-chart>
<hc-chart type="donut" height="180" label="Time">Design: 12\nBuild: 30\nReview: 7</hc-chart>
<hc-gauge label="Capacity" value="72" unit="%"></hc-gauge>
<hc-sparkline label="Trend">Mon: 12\nTue: 15\nWed: 11\nThu: 18</hc-sparkline>
</section></div>
<hc-timeline label="Schedule">2026-10-26 | Design | 2026-10-27\n2026-10-28 | Build | 2026-10-30\n2026-10-31 | Review</hc-timeline>
<hc-heatmap label="Daily work" start="2026-10-01" end="2026-10-31">2026-10-01: 0\n2026-10-02: 1\n2026-10-03: 3\n2026-10-04: 6\n2026-10-05: 9</hc-heatmap>
<hc-flow label="Review route" direction="right">[Work]\nsource: Sources\nbuild: Build {info}\n[Decision]\nreview: Review {success}\nsource -> build -> review</hc-flow>`;
const ask = `<h1>Choose the next batch</h1><hc-ask title="Batch preferences" send="Use these answers">
<hc-choice name="scope" label="Scope">* Small batch\nWhole project</hc-choice>
<hc-multi name="areas" label="Include">* Notes\nTasks\nCalendar</hc-multi>
<hc-range name="hours" label="Hours" min="0" max="40" value="12" unit="h"></hc-range>
<hc-rank name="order" label="Order">Design\nBuild\nReview</hc-rank>
<hc-matrix name="priority" label="Priority" options="Low | Medium | High">Notes\nTasks</hc-matrix>
<hc-pick name="batch" label="Batch size">Small: 5\nMedium: 12\nLarge: 30</hc-pick>
<hc-text name="note" label="Notes" placeholder="Add context"></hc-text>
<hc-date name="day" label="Review date" value="2026-10-31"></hc-date></hc-ask>`;

for (const theme of ['light', 'dark']) for (const kind of ['sample', 'ask']) {
  test(`${kind} dither in ${theme}`, async ({ page }) => {
    const errors: string[] = [];
    page.on('pageerror', e => errors.push(e.message));
    await page.setViewportSize({ width: 1440, height: 920 });
    await page.emulateMedia({ reducedMotion: 'no-preference', colorScheme: theme as 'light' | 'dark' });
    const styles = read('src/styles/tokens.css');
    const aliases = [...read('src/components/htmlSandbox.ts').matchAll(/\['(--app-[\w-]+)', '(--[\w-]+)'/g)].map(m => `${m[1]}:var(${m[2]})`).join(';');
    const kit = read('src/canvas/kit/generated/kit.min.js'), css = read('src/canvas/kit/generated/kit.min.css');
    const askJs = read('src/canvas/kit/generated/kit-ask.min.js'), askCss = read('src/canvas/kit/generated/kit-ask.min.css');
    const html = `<html data-theme="${theme}"><head><base href="http://127.0.0.1:1420/"><style>${styles}\n:root{${aliases}}body{margin:0;padding:24px;font:15px/1.55 system-ui;background:var(--surface-card);color:var(--ink)}h1{font-size:26px;margin:0 0 16px}.sample-grid{display:grid;grid-template-columns:1fr 1fr;gap:24px}section{min-width:0}hc-ask{max-width:850px;margin:auto}</style><style>${css}${askCss}</style>
    <script>window.__start=performance.now();window.__allowed=true;window.__level='full';window.__encodes=0;const encode=HTMLCanvasElement.prototype.toDataURL;HTMLCanvasElement.prototype.toDataURL=function(){if(this.width<=32&&this.height<=32)window.__encodes++;return encode.apply(this,arguments)};window.hollowCanvas=Object.freeze({init:{},get textureAllowed(){return window.__allowed},get textureLevel(){return window.__level},get(){},set(){return true}});</script>
    <script>${kit}</script><script>${askJs}</script></head><body>${kind === 'ask' ? ask : sample}<script>document.addEventListener('DOMContentLoaded',()=>{document.body.getBoundingClientRect();window.__render=performance.now()-window.__start;});</script></body></html>`;
    const times: number[] = [];
    const cpu: number[] = [];
    for (let i = 0; i < 6; i++) {
      await page.goto('about:blank');
      await page.setContent(html);
      const ms = await page.evaluate(() => {
        const start = performance.now();
        document.body.getBoundingClientRect();
        return new Promise<number>(r => requestAnimationFrame(() => requestAnimationFrame(() => r(performance.now() - start))));
      });
      times.push(ms);
      cpu.push(await page.evaluate(() => (window as unknown as { __render: number }).__render));
    }
    await expect(page.locator('hc-stat, hc-ask').first()).toHaveAttribute('data-hc-ready', '');
    if (kind === 'sample') await page.locator('.hc-sort').nth(1).click();
    // Finish existing finite entrances before capturing every chart, including
    // those below the viewport. This does not change the texture policy.
    await page.evaluate(() => {
      document.documentElement.dataset.hcMotion = 'reduce';
      dispatchEvent(new Event('hc-reflow')); dispatchEvent(new Event('resize'));
      return new Promise<void>(r => requestAnimationFrame(() => requestAnimationFrame(() => r())));
    });
    await expect(page.locator('.hc-chart-entrance')).toHaveCount(0);
    const folder = process.env.DITHER_CAPTURES;
    if (folder) {
      mkdirSync(folder, { recursive: true });
      const prefix = `${process.env.DITHER_PHASE || 'after'}-${kind}-${theme}`;
      await page.screenshot({ path: resolve(folder, `${prefix}.png`), fullPage: true });
      writeFileSync(resolve(folder, `${prefix}.json`), JSON.stringify({ theme, kind, renderCpuMs: cpu, layoutAndTwoFramesMs: times }, null, 2));
    }
    if (process.env.DITHER_PHASE !== 'before') {
      await expect(page.locator('html')).toHaveAttribute('data-hc-material', 'dither');
      if (kind === 'sample') {
        for (const name of ['hc-gauge', 'hc-heatmap', 'hc-flow', 'hc-timeline']) {
          const svg = page.locator(`${name} svg`).first();
          const textured = await svg.screenshot();
          const hidden = await page.addStyleTag({ content: '.hc-stipple{visibility:hidden!important}' });
          expect((await svg.screenshot()).equals(textured), `${name} paints its shared SVG pattern`).toBe(false);
          await hidden.evaluate(el => el.remove());
        }
        await page.evaluate(() => scrollTo(0, 0));
      }
      const rects = await page.evaluate(() => {
        const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
        const out: { text: string; need: 3 | 4.5; x: number; y: number; width: number; height: number }[] = [];
        while (walker.nextNode()) {
          const node = walker.currentNode, el = node.parentElement;
          if (!el || !node.textContent?.trim() || el.closest('script,style,[hidden],.hc-sr,title')) continue;
          const range = document.createRange(); range.selectNodeContents(node);
          const r = range.getBoundingClientRect();
          if (r.width < 3 || r.height < 3 || !el.checkVisibility()) continue;
          out.push({ text: node.textContent.trim().slice(0, 60), need: el.closest('.hc-icon,.hc-step-mark,.hc-checklist-mark,.hc-sw,.hc-file-icon,.hc-rank-grip') ? 3 : 4.5, x: Math.floor(r.x), y: Math.floor(r.y + scrollY), width: Math.ceil(r.width + 1), height: Math.ceil(r.height + 1) });
        }
        return out;
      });
      const pictures: string[] = [];
      pictures.push((await page.screenshot({ fullPage: true })).toString('base64'));
      for (const colour of ['transparent', 'black', 'white']) {
        const style = await page.addStyleTag({ content: `body *{-webkit-text-fill-color:${colour}!important}svg text,svg tspan{fill:${colour}!important}` });
        pictures.push((await page.screenshot({ fullPage: true })).toString('base64'));
        await style.evaluate(el => el.remove());
      }
      const decoder = await page.context().newPage();
      const decoded = await decoder.evaluate(async ({ pictures, rects }) => {
        return Promise.all(pictures.map(async b64 => {
          const image = new Image(); image.src = `data:image/png;base64,${b64}`; await image.decode();
          const c = document.createElement('canvas'); c.width = image.width; c.height = image.height;
          const ctx = c.getContext('2d')!; ctx.drawImage(image, 0, 0);
          return rects.map(r => ({ width: r.width, height: r.height, data: Array.from(ctx.getImageData(r.x, r.y, r.width, r.height).data) }));
        }));
      }, { pictures, rects });
      await decoder.close();
      const pixel = (i: number, j: number): Pixels => ({ ...decoded[i][j], data: new Uint8ClampedArray(decoded[i][j].data) });
      const verdicts = rects.map((r, j) => ({ text: r.text, ...scoreBox({ shown: pixel(0,j), hidden: pixel(1,j), black: pixel(2,j), white: pixel(3,j) }, r.need) }));
      const texturedText = rects.filter((r, j) => {
        if (r.need !== 4.5) return false;
        const colours = new Set<string>();
        const hidden = decoded[1][j].data, black = decoded[2][j].data, white = decoded[3][j].data;
        for (let i = 0; i < hidden.length; i += 4) {
          if (Math.min(white[i]-black[i], white[i+1]-black[i+1], white[i+2]-black[i+2]) < 192) continue;
          colours.add(hidden.slice(i, i + 3).join(','));
        }
        return colours.size > 1;
      });
      expect(texturedText.map(r => r.text), 'glyphs have solid plates').toEqual([]);
      const measured = verdicts.filter(v => v.kind === 'measured');
      expect(measured.length).toBeGreaterThan(15);
      const failures = measured.filter(v => v.ratio < v.need);
      if (folder) writeFileSync(resolve(folder, `contrast-${kind}-${theme}.json`), JSON.stringify(verdicts, null, 2));
      expect(failures).toEqual([]);
      expect(measuredFindings(measured.map(v => ({ rule: 'texture-text' as const, text: v.text, need: v.need, box: [0,0,20,20] as [number,number,number,number] })), new Map(measured.map((v,i) => [i,v])))).toEqual([]);
      const marks = await page.evaluate(() => {
        const cs = getComputedStyle(document.documentElement);
        const probe = document.createElement('span'); document.body.append(probe);
        const rgb = (v: string) => { probe.style.color = v; return getComputedStyle(probe).color.match(/[\d.]+/g)!.slice(0, 3).map(Number); };
        const surface = rgb(cs.getPropertyValue('--hc-surface'));
        const rows = ['--hc-primary','--hc-muted','--hc-ink','--hc-s1'].map(n => ({ name: n, ink: rgb(cs.getPropertyValue(n)), surface }));
        document.querySelectorAll('.hc-text-input,.hc-range-number,.hc-date-input').forEach(el => {
          const style = getComputedStyle(el);
          rows.push({ name: el.className, ink: rgb(style.borderColor), surface: rgb(style.backgroundColor) });
        });
        probe.remove(); return rows;
      });
      expect(marks.filter(m => contrastRatio(m.ink, m.surface) < 3)).toEqual([]);
      if (kind === 'sample') {
        expect(await page.locator('hc-callout').evaluate(el => {
          const p = document.createElement('p'); p.textContent = 'Nested note'; el.append(p);
          const colour = getComputedStyle(p).backgroundColor; p.remove(); return colour;
        })).toBe('rgba(0, 0, 0, 0)');
      }
      const encodes = () => page.evaluate(() => (window as unknown as { __encodes: number }).__encodes);
      expect(await encodes()).toBe(1);
      await page.evaluate(() => { scrollTo(0, 500); dispatchEvent(new Event('resize')); });
      expect(await encodes()).toBe(1);
      await page.evaluate(next => { document.documentElement.dataset.theme = next; dispatchEvent(new Event('hollow-theme')); }, theme === 'light' ? 'dark' : 'light');
      expect(await encodes()).toBe(2);
      await page.evaluate(next => { document.documentElement.dataset.theme = next; dispatchEvent(new Event('hollow-theme')); scrollTo(0, 0); }, theme);
      expect(await encodes()).toBe(3);
      // Density and media changes preserve controls, selections and focus.
      if (kind === 'ask') await page.getByLabel('Whole project', { exact: true }).check();
      const full = await page.locator('html').evaluate(el => el.style.getPropertyValue('--hc-dither-mask'));
      await page.evaluate(() => { (window as unknown as { __level: string }).__level = 'quiet'; dispatchEvent(new Event('hollow-texture')); });
      await expect(page.locator('html')).toHaveAttribute('data-hc-density', 'quiet');
      expect(await page.locator('html').evaluate(el => el.style.getPropertyValue('--hc-dither-mask'))).not.toBe(full);
      await page.evaluate(() => { (window as unknown as { __allowed: boolean }).__allowed = false; dispatchEvent(new Event('hollow-texture')); });
      await expect(page.locator('html')).toHaveAttribute('data-hc-material', 'plain');
      if (kind === 'ask') await expect(page.getByLabel('Whole project', { exact: true })).toBeChecked();
      await page.evaluate(() => { (window as unknown as { __allowed: boolean; __level: string }).__allowed = true; (window as unknown as { __level: string }).__level = 'full'; dispatchEvent(new Event('hollow-texture')); });
      await page.emulateMedia({ reducedMotion: 'reduce' });
      await expect(page.locator('html')).toHaveAttribute('data-hc-material', 'plain');
      await page.emulateMedia({ reducedMotion: 'no-preference', contrast: 'more' });
      await expect(page.locator('html')).toHaveAttribute('data-hc-material', 'plain');
      await page.emulateMedia({ contrast: 'no-preference' });
      if (kind === 'sample' && theme === 'light' && process.env.DITHER_PHASE !== 'before') {
        await page.emulateMedia({ forcedColors: 'active' });
        await expect(page.locator('html')).toHaveAttribute('data-hc-material', 'plain');
        if (folder) await page.screenshot({ path: resolve(folder, 'after-forced-colours.png'), fullPage: true });
      }
    }
    expect(errors).toEqual([]);
  });
}
