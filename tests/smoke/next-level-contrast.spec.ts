import { test, expect, type Page } from './fixtures/test';

// next-level P21: every text token is readable on every surface it is used
// on, in light and dark (WCAG 2.x contrast from the browser's own resolved
// colours, so color-mix() and theme overrides are what the user sees).
// Body text: 4.5:1. --muted-soft is for de-emphasised metadata and
// placeholders only, so it is held to 3:1 (large/UI-component level).
const SURFACES = ['--canvas', '--surface-card', '--surface-raised', '--surface-soft', '--surface-sunken'];
const TEXT: [string, number][] = [['--ink', 4.5], ['--body-strong', 4.5], ['--body', 4.5], ['--muted', 4.5], ['--muted-soft', 3], ['--error', 4.5], ['--risk', 4.5]];
const ON_PRIMARY: [string, string, number] = ['--on-primary', '--primary', 4.5];

async function pairs(page: Page, theme: 'light' | 'dark') {
  return page.evaluate(({ theme, SURFACES, TEXT, ON_PRIMARY }) => {
    document.documentElement.dataset.theme = theme;
    const probe = document.createElement('div');
    document.body.appendChild(probe);
    const rgb = (v: string, prop: 'color' | 'backgroundColor') => {
      probe.style.color = ''; probe.style.backgroundColor = '';
      probe.style[prop] = `var(${v})`;
      const c = getComputedStyle(probe)[prop];
      const m = c.match(/[\d.]+/g)!.map(Number);
      // color(srgb r g b) is 0–1; rgb() is 0–255.
      const [r, g, b] = c.startsWith('color(') ? m.slice(0, 3).map(x => x * 255) : m.slice(0, 3);
      return { r, g, b, a: m.length > 3 ? m[3] : 1 };
    };
    const lum = ({ r, g, b }: { r: number; g: number; b: number }) => {
      const f = (x: number) => { x /= 255; return x <= 0.03928 ? x / 12.92 : ((x + 0.055) / 1.055) ** 2.4; };
      return 0.2126 * f(r) + 0.7152 * f(g) + 0.0722 * f(b);
    };
    const ratio = (fg: string, bg: string) => {
      const b = rgb(bg, 'backgroundColor'); const f = rgb(fg, 'color');
      // Composite a translucent foreground over the surface.
      const mix = { r: f.r * f.a + b.r * (1 - f.a), g: f.g * f.a + b.g * (1 - f.a), b: f.b * f.a + b.b * (1 - f.a) };
      const [l1, l2] = [lum(mix), lum(b)].sort((x, y) => y - x);
      return +((l1 + 0.05) / (l2 + 0.05)).toFixed(2);
    };
    const out: { fg: string; bg: string; ratio: number; min: number }[] = [];
    for (const bg of SURFACES) for (const [fg, min] of TEXT) out.push({ fg, bg, ratio: ratio(fg, bg), min });
    out.push({ fg: ON_PRIMARY[0], bg: ON_PRIMARY[1], ratio: ratio(ON_PRIMARY[0], ON_PRIMARY[1]), min: ON_PRIMARY[2] });
    probe.remove();
    return out;
  }, { theme, SURFACES, TEXT, ON_PRIMARY });
}

for (const theme of ['light', 'dark'] as const) {
  test(`text tokens meet contrast on every surface (${theme})`, async ({ page }, info) => {
    test.skip(info.project.name === 'chromium-narrow', 'colour, not layout');
    await page.goto('/?mock=1');
    await page.getByRole('navigation', { name: 'Main' }).waitFor();
    const result = await pairs(page, theme);
    const failing = result.filter(p => p.ratio < p.min).map(p => `${p.fg} on ${p.bg}: ${p.ratio} < ${p.min}`);
    console.log(`${theme}: ${result.length} pairs, lowest ${Math.min(...result.map(p => p.ratio))}`);
    expect(failing).toEqual([]);
  });
}
