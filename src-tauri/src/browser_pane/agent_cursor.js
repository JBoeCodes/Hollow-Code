(() => {
  const data = __PAYLOAD__;
  const key = '__hollowInspectionMarker';
  const old = globalThis[key];
  const clear = () => { const current = globalThis[key]; if (current) current.dispose(); };
  if (!data || location.href !== data.url || innerWidth !== data.point.width || innerHeight !== data.point.height
      || Math.round(scrollX) !== data.point.scroll_x || Math.round(scrollY) !== data.point.scroll_y || data.remaining <= 0) {
    clear(); return 'cleared';
  }
  const reduced = matchMedia('(prefers-reduced-motion: reduce)');
  const forced = matchMedia('(forced-colors: active)');
  const duration = reduced.matches || forced.matches ? 0 : data.duration;
  if (old && old.seq === data.seq) {
    old.dot.style.transitionDuration = `${duration}ms`;
    return 'kept';
  }
  const previous = old ? getComputedStyle(old.dot).transform : undefined;
  clear();
  const host = document.createElement('div');
  host.setAttribute('aria-hidden', 'true');
  host.style.setProperty('all', 'initial', 'important');
  for (const [name, value] of Object.entries({position:'fixed', inset:'0', 'z-index':'2147483647', 'pointer-events':'none', overflow:'hidden'})) host.style.setProperty(name, value, 'important');
  const shadow = host.attachShadow({mode:'closed'});
  const dot = document.createElement('div');
  Object.assign(dot.style, {position:'absolute', left:'0', top:'0', pointerEvents:'none', display:'flex', alignItems:'start', gap:'4px', font:'12px system-ui', color:forced.matches ? 'CanvasText' : data.ink});
  const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
  svg.setAttribute('width', '18'); svg.setAttribute('height', '22'); svg.setAttribute('viewBox', '0 0 18 22');
  const path = document.createElementNS('http://www.w3.org/2000/svg', 'path');
  path.setAttribute('d', 'M1 1 L1 17 L5 13 L9 21 L12 19 L8 11 L15 11 Z');
  path.setAttribute('fill', forced.matches ? 'Highlight' : data.color);
  path.setAttribute('stroke', forced.matches ? 'Canvas' : data.background);
  svg.append(path);
  const label = document.createElement('span');
  label.textContent = 'Agent inspected';
  Object.assign(label.style, {background:forced.matches ? 'Canvas' : data.background, padding:'3px 5px', border:'1px solid currentColor', borderRadius:'4px', whiteSpace:'nowrap'});
  dot.append(svg, label); shadow.append(dot); document.documentElement.append(host);
  const position = `translate(${data.point.x}px, ${data.point.y}px)`;
  dot.style.transform = duration && previous ? previous : position;
  // The pointer stays at the inspected coordinate. The caption can flip to
  // the left near the right edge without moving that coordinate.
  if (data.point.x > innerWidth - 160) { label.style.position = 'absolute'; label.style.right = '20px'; }
  if (data.point.y > innerHeight - 30) label.style.transform = 'translateY(-24px)';
  let frame = 0;
  if (duration && previous) {
    dot.style.transitionProperty = 'transform'; dot.style.transitionDuration = `${duration}ms`; dot.style.transitionTimingFunction = 'ease-out';
    frame = requestAnimationFrame(() => { dot.style.transform = position; });
  }
  const stopMotion = () => { dot.style.transitionDuration = '0ms'; dot.style.transform = position; };
  // pushState and replaceState do not emit pagehide. Observe without
  // replacing any page API or dispatching an event into the page.
  let watch = 0;
  const checkLocation = () => {
    if (location.href !== data.url) { clear(); return; }
    watch = requestAnimationFrame(checkLocation);
  };
  const dispose = () => {
    cancelAnimationFrame(frame); cancelAnimationFrame(watch); clearTimeout(timer); host.remove();
    removeEventListener('scroll', clear, true); removeEventListener('resize', clear); removeEventListener('pagehide', clear);
    reduced.removeEventListener('change', stopMotion); forced.removeEventListener('change', clear);
    if (globalThis[key]?.host === host) delete globalThis[key];
  };
  const timer = setTimeout(clear, data.remaining);
  globalThis[key] = {seq:data.seq, host, dot, position, dispose};
  watch = requestAnimationFrame(checkLocation);
  addEventListener('scroll', clear, true); addEventListener('resize', clear); addEventListener('pagehide', clear);
  reduced.addEventListener('change', stopMotion); forced.addEventListener('change', clear);
  return 'shown';
})()
