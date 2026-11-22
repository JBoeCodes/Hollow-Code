/** One static threshold tile per frame, shared by CSS and SVG. No scroll work. */
import { textureOf, watchTextures } from './texture';
import { KIT_COMPONENTS } from './manifest';

let tile: HTMLCanvasElement | undefined, key = '', generation = 0, picture = '';
/** CSS, SVG and every chart reuse this one device-pixel-snapped source. */
export function materialTile(): { tile: HTMLCanvasElement; generation: number; size: number; quiet: boolean } | null {
  const root = document.documentElement;
  const quiet = (window as unknown as { hollowCanvas?: { textureLevel?: string } }).hollowCanvas?.textureLevel === 'quiet';
  const dpr = Math.max(1, Math.min(4, devicePixelRatio || 1));
  const ink = getComputedStyle(root).getPropertyValue('--hc-muted').trim();
  const next = `${dpr}:${quiet}:${ink}`;
  tile ??= document.createElement('canvas');
  const pen = tile.getContext('2d');
  if (!pen) return null;
  if (next !== key) {
    key = next; generation++;
    const cell = Math.max(1, Math.round(2 * dpr));
    tile.width = tile.height = cell * 4;
    pen.clearRect(0, 0, tile.width, tile.height);
    pen.fillStyle = ink;
    const bayer = [0,8,2,10,12,4,14,6,3,11,1,9,15,7,13,5];
    for (let i = 0; i < 16; i++) if (bayer[i] < (quiet ? 2 : 6)) pen.fillRect((i % 4) * cell, Math.floor(i / 4) * cell, Math.max(1, Math.round(dpr)), Math.max(1, Math.round(dpr)));
    picture = tile.toDataURL();
  }
  return { tile, generation, size: tile.width / dpr, quiet };
}

export function material(): void {
  const mount = () => {
    if (!document.querySelector(['hc-canvas', '[data-hc-texture-root]', 'hc-ask', ...KIT_COMPONENTS].join(','))) return;
    const root = document.documentElement;
    if (root.hasAttribute('data-hc-material')) return;
    let seen = 0;
    const update = () => {
      const owner = document.querySelector('hc-canvas,[data-hc-texture-root]') || document.body;
      root.dataset.hcMaterial = textureOf(owner);
      if (root.dataset.hcMaterial === 'plain') return;
      const source = materialTile();
      if (!source) { root.dataset.hcMaterial = 'plain'; return; }
      updatePatterns();
      if (source.generation === seen) return;
      seen = source.generation;
      root.style.setProperty('--hc-dither-mask', `url("${picture}")`);
      root.style.setProperty('--hc-dither-size', `${source.size}px`);
      root.dataset.hcDensity = source.quiet ? 'quiet' : 'full';
    };
    watchTextures(update);
    addEventListener('resize', update);
    addEventListener('hollow-theme', update);
    addEventListener('hc-reflow', updatePatterns);
    const dprChange = () => {
      matchMedia(`(resolution: ${devicePixelRatio}dppx)`).addEventListener('change', () => { update(); dprChange(); }, { once: true });
    };
    dprChange();
    update();
  };
  if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', mount, { once: true });
  else mount();
}

const SVG = 'http://www.w3.org/2000/svg';
let ids = 0;
function updatePattern(svg: SVGSVGElement): void {
  if (textureOf(svg) !== 'dither') return;
  const source = materialTile(), pattern = svg.querySelector<SVGPatternElement>('[data-hc-pattern]');
  if (!source || !pattern) return;
  const matrix = svg.getScreenCTM();
  const x = source.size / (matrix ? Math.hypot(matrix.a, matrix.b) || 1 : 1);
  const y = source.size / (matrix ? Math.hypot(matrix.c, matrix.d) || 1 : 1);
  for (const el of [pattern, pattern.firstElementChild!]) {
    el.setAttribute('width', String(x)); el.setAttribute('height', String(y));
  }
  pattern.firstElementChild!.setAttribute('href', picture);
}
function updatePatterns(): void {
  document.querySelectorAll<SVGSVGElement>('svg[data-hc-pattern-owner]').forEach(updatePattern);
}

/** Texture stays in the interior; exact edges and labels keep their own ink. */
export function stipple(svg: SVGElement, shape: SVGElement, cls = ''): void {
  const owner = svg instanceof SVGSVGElement ? svg : svg.ownerSVGElement;
  // Groups inherit the pattern reference from their owning SVG.
  if (owner && !owner.hasAttribute('data-hc-pattern-owner')) {
    owner.setAttribute('data-hc-pattern-owner', '');
    const pattern = document.createElementNS(SVG, 'pattern'), image = document.createElementNS(SVG, 'image');
    pattern.id = `hc-material-${++ids}`;
    pattern.setAttribute('data-hc-pattern', ''); pattern.setAttribute('patternUnits', 'userSpaceOnUse');
    pattern.append(image); owner.prepend(pattern);
    owner.style.setProperty('--hc-svg-stipple', `url(#${pattern.id})`);
    queueMicrotask(() => { if (owner.isConnected) updatePattern(owner); });
  }
  const copy = shape.cloneNode(false) as SVGElement;
  copy.removeAttribute('id');
  copy.setAttribute('class', `hc-stipple ${cls}`);
  copy.setAttribute('aria-hidden', 'true');
  copy.setAttribute('pointer-events', 'none');
  svg.append(copy);
}
