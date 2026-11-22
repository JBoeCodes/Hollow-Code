import { Match, Switch } from 'solid-js';
import type { EmptyScene } from '../emptyScene';
import '../styles/empty-art.css';
/** Decorative architectural sketches. Meaning and actions remain in the surrounding text. */
export default function EmptyArtDrawing(props: { kind: EmptyScene; compact?: boolean }) {
  return <span class="empty-art" classList={{compact:props.compact}} data-scene={props.kind} aria-hidden="true">
    <svg viewBox="0 0 160 112" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
      <rect class="empty-art-plate" x="22" y="12" width="116" height="88" rx="3"/>
      <path class="empty-art-grid" d="M22 76h116M42 12v88m76-88v88"/>
      <path class="empty-art-edge" d="M39 13h82M23 29v16"/>
      <path class="empty-art-ground" d="M24 92H136M36 97H124"/>
      <g class="empty-art-scene"><Switch>
        <Match when={props.kind === 'tasks'}><path d="M48 87V30h64v57M61 30v-8h38v8M57 42h8v8h-8zM57 61h8v8h-8zM75 46h26M75 65h20"/><path class="empty-art-accent" d="m56 80 5 5 10-12"/></Match>
        <Match when={props.kind === 'automations'}><path d="M38 76V47h24v29zM98 76V47h24v29zM62 61h36M72 61V35a8 8 0 0 1 16 0v26M50 47V35h16M110 76v9H86"/><path class="empty-art-accent" d="m80 80 6 5-6 5M43 56h14M103 56h14"/></Match>
        <Match when={props.kind === 'files'}><path d="M34 85h92V50H34zM45 50V26h32l12 12v12M77 26v12h12M61 50V40h39l14 14v31M100 40v14h14"/><path class="empty-art-accent" d="M45 66h26m-26 8h42"/></Match>
        <Match when={props.kind === 'visuals'}><path d="M32 33h96v54H32zM43 24h74v9M52 16h56v8"/><path class="empty-art-accent" d="M44 73V59h12v14m10 0V48h12v25m10 0V54h12v19M43 79h70"/></Match>
        <Match when={props.kind === 'inbox'}><path d="m39 61 10-22h62l10 22v25H39zM39 61h25l5 8h22l5-8h25M49 39l8 22m54-22-8 22"/><path class="empty-art-accent" d="M68 51V33a12 12 0 0 1 24 0v18M74 51V34a6 6 0 0 1 12 0v17"/></Match>
        <Match when={props.kind === 'commitments'}><path d="M39 87V48a20 20 0 0 1 40 0v39M81 87V48a20 20 0 0 1 40 0v39M46 87V49a13 13 0 0 1 26 0v38M88 87V49a13 13 0 0 1 26 0v38"/><path class="empty-art-accent" d="M61 64h38m-6-6 6 6-6 6m-26-12-6 6 6 6"/></Match>
        <Match when={props.kind === 'results'}><path d="M42 81h76v9H42zM53 81V25h42l13 13v43M95 25v13h13M63 48h32M63 56h23M63 64h29"/><path class="empty-art-accent" d="M116 27v12m-6-6h12M124 48v6m-3-3h6"/></Match>
        <Match when={props.kind === 'sources'}><path d="m38 38 15-8 16 55-15 5zM58 26h18v61H58M85 32h18v55H85m-23-51h10m-10 7h10m-10 33h10m17-34h10m-10 35h10M36 90h90"/><path class="empty-art-accent" d="m109 30 13 2-5 55-13-2zM112 42l6 1"/></Match>
        <Match when={props.kind === 'mission'}><ellipse cx="80" cy="59" rx="53" ry="24"/><ellipse cx="80" cy="59" rx="34" ry="14"/><path d="M70 69V52a10 10 0 0 1 20 0v17zM75 69V53a5 5 0 0 1 10 0v16"/><g class="empty-art-accent"><circle cx="125" cy="46" r="4"/><path d="M40 24v8m-4-4h8"/></g></Match>
        <Match when={props.kind === 'search'}><circle cx="71" cy="50" r="25"/><circle cx="71" cy="50" r="19"/><path d="m90 67 27 24 6-7-27-23"/><path class="empty-art-accent" d="M65 59V46a6 6 0 0 1 12 0v13M107 28h10m-5-5v10"/></Match>
        <Match when={props.kind === 'waiting'}><path d="M64 87V46a16 16 0 0 1 32 0v41M70 87V47a10 10 0 0 1 20 0v40M32 78h22m52 0h22M43 69h11m52 0h11"/><g class="empty-art-accent"><circle cx="112" cy="32" r="8"/><path d="M108 32h8m-4-4v8"/></g></Match>
      </Switch></g>
    </svg>
  </span>;
}
