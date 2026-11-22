/** Original scalar fields from Hollow's folded/orbital threshold study.
 * Four bounded cached variants; no time, random state, DOM or frame loop. */
export type ThresholdKind = 'folded' | 'orbital';
const cache = new Map<string, readonly string[]>();
function arch(x: number, y: number, r: number, bottom: number) {
  return y < 0 ? Math.abs(Math.hypot(x, y) - r) : y <= bottom ? Math.abs(Math.abs(x) - r) : Math.hypot(Math.abs(x) - r, y - bottom);
}
function field(x: number, y: number, kind: ThresholdKind) {
  if (kind === 'orbital') {
    const r = Math.hypot(x,y);
    if (r > .82) return 0;
    if ((Math.abs(x)<.15 && y>-.16 && y<.34) || (Math.hypot(x,y+.16)<.15 && y<=-.16)) return 0;
    const z = Math.sqrt(Math.max(0,1-(r/.71)**2));
    const edge = Math.exp(-(((r-.7)/.055)**2));
    return Math.min(.98,(r<.71 ? .14+.52*z*Math.max(.12,(1-x-y)*.42)+(.5+.5*Math.cos(y*39+z*5))**7*.27 : 0)+edge*.55);
  }
  let tone=0;
  for (let k=0;k<4;k++) {
    const d=arch(x-(k-1.5)*.085,y+(k-1.5)*.075+.1,.43+k*.018,.58);
    tone=Math.max(tone,Math.exp(-d*d/(.0018+k*.0004))*(.82-k*.11)+Math.exp(-d*d/.018)*.18);
  }
  return Math.min(.96,tone*Math.max(0,Math.min(1,(1.02-Math.abs(x))*2)));
}
export function thresholdRows(kind: ThresholdKind, quiet = false): readonly string[] {
  const key=`${kind}:${quiet}`;
  const saved=cache.get(key);
  if (saved) return saved;
  const glyphs=' .:+*oO#@';
  const rows=Array.from({length:36},(_,row)=>Array.from({length:80},(_,col)=>{
    const tone=field((col-39.5)/27,(row-15)/19,kind);
    if (tone < (quiet ? .26 : .065)) return ' ';
    return glyphs[Math.min(8,Math.max(1,Math.floor(tone*8)))];
  }).join(''));
  const result=Object.freeze(rows);
  cache.set(key,result);
  return result;
}
