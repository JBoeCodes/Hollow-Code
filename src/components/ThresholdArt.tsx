import { For } from 'solid-js';
import { thresholdRows, type ThresholdKind } from '../thresholdField';
import { studioTexture } from '../store/studio';
import '../styles/threshold-art.css';

/** Decorative only. One SVG text run per row, never one DOM node per glyph. */
export default function ThresholdArt(props: { kind?: ThresholdKind; quiet?: boolean }) {
  return <svg class="threshold-art" viewBox="0 0 600 300" aria-hidden="true">
    <g class="threshold-art-field" fill="currentColor">
      <For each={thresholdRows(props.kind ?? 'folded', props.quiet || studioTexture() === 'quiet')}>{(row,i)=><text x="60" y={16+i()*7.5} textLength="480" lengthAdjust="spacingAndGlyphs">{row}</text>}</For>
    </g>
    <g class="threshold-art-plain" fill="none" stroke="currentColor" stroke-width="2">
      <path d="M230 250V120a70 70 0 0 1 140 0v130M250 250V120a50 50 0 0 1 100 0v130M220 250h160"/>
    </g>
  </svg>;
}
