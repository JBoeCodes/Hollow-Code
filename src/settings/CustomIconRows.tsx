import { For, Show, createEffect, createSignal, onMount, onCleanup } from 'solid-js';
import { api } from '../api';
import { setIconEditor, useSavedIcon } from '../store/customIcons';
import { onNative } from '../nativeEvents';
import { trackTask } from '../store/activityDock';
import { showConfirm } from '../store/overlays';
import { MainAgentIndicator } from '../components/SessionIndicator';
import { ErrorText } from '../components/ui';
import stock from '../assets/brand/hollow-mark-96.png';
import type { CustomIcon, IconTarget } from '../customIcons';
import { APP_ICON_PRESETS, appIconLabel, selectedAppIcon, type AppIconPreset } from '../appIconPresets';
import { errMsg } from '../utils';
import '../styles/custom-icons.css';

// The rows sit in a closed fold (Settings, General, App and COO icons), so an error or a notice in
// one (a damaged saved icon, "The saved icon changed") would stay out of sight until the fold is opened. The
// fold's summary says "Needs attention" while any row has one (overnight 11-07, lane D3; the fold's own open
// state stays the person's: nothing opens or closes it).
const [flagged,setFlagged]=createSignal<readonly IconTarget[]>([]);
export const iconRowsNeedAttention=()=>flagged().length>0;
export default function CustomIconRows() {
  return <div class="custom-icon-rows"><For each={['app','mainAgent'] as IconTarget[]}>{target=><IconRow target={target}/>}</For></div>;
}
function IconRow(props: {target: IconTarget}) {
  const [record,setRecord]=createSignal<CustomIcon|null>(null), [error,setError]=createSignal(''), [loading,setLoading]=createSignal(true), [busy,setBusy]=createSignal(false);
  const [known,setKnown]=createSignal(false);
  const [readFailed,setReadFailed]=createSignal(false), [notice,setNotice]=createSignal('');
  createEffect(()=>{const needs=!!error()||!!notice();setFlagged(prev=>needs===prev.includes(props.target)?prev:needs?[...prev,props.target]:prev.filter(t=>t!==props.target));});
  onCleanup(()=>setFlagged(prev=>prev.filter(t=>t!==props.target)));
  const isApp=()=>props.target==='app';
  const title=()=>isApp()?'App icon':'COO icon';
  const resetLabel=()=>isApp()?'Reset to Fold':'Reset to default';
  const damaged=()=>readFailed()&&error().startsWith('ICON_DAMAGED:');
  let request=0, disposed=false;
  async function read() {
    const id=++request; setLoading(true);setNotice('');
    try { const r=await api.readCustomIcon(props.target); if(!disposed&&id===request){setRecord(r);setKnown(true);setReadFailed(false);setError('');useSavedIcon(props.target,r);} }
    catch(e){if(!disposed&&id===request){setKnown(false);setReadFailed(true);setError(errMsg(e));}}
    finally{if(!disposed&&id===request)setLoading(false);}
  }
  onMount(()=>{void read();onCleanup(onNative<string>('custom-icon-changed',t=>{if(t===props.target)void read();}));});
  onCleanup(()=>{disposed=true;request++;});
  async function change(preset?:AppIconPreset) {
    if(busy()||loading()||(preset&&!known()))return;
    setBusy(true);setNotice('');
    const before = record(), revision = known() ? (before?.revision ?? null) : null;
    request++; // A pre-mutation read must never replace its result.
    try {
      if (before && !before.preset && !await showConfirm({title: 'Replace your custom icon?', body: 'Its original is deleted.', confirmLabel: preset ? 'Replace icon' : resetLabel(), danger: true})) return;
      if (disposed) return;
      await trackTask(preset?'Change app icon':'Reset icon',()=>preset
        ?api.selectAppIconPreset(preset,revision)
        :api.resetCustomIcon(props.target,revision));
      // Another window may have committed after this command but before its
      // response arrived. Read current storage; never replay that old result.
      if(!disposed)await read();
    } catch(e) {
      if(!disposed){
        if(errMsg(e).startsWith('ICON_CONFLICT:')){
          await read();
          if(!disposed&&known())setNotice('The saved icon changed. The current selection is shown; nothing was replaced.');
        }else{setReadFailed(false);setError(errMsg(e));}
      }
    } finally {if(!disposed)setBusy(false);}
  }
  return <section class="custom-icon-row" classList={{'custom-icon-app':isApp()}} aria-label={title()}>
    <div class="custom-icon-current" aria-hidden="true"><Show when={record()?.pngs[128]} fallback={isApp()?<img src={stock} alt=""/>:<MainAgentIndicator state="resting" size={56} decorative/>}>{png=><img src={`data:image/png;base64,${png()}`} alt=""/>}</Show></div>
    <div class="custom-icon-copy"><h3>{title()}</h3><p>{isApp()?'Shows in Hollow, the menu bar and the Dock while Hollow is open.':'Your COO, wherever it appears.'}</p>
      <Show when={isApp()}>
        <div class="app-icon-presets" role="group" aria-label="App icon presets" aria-busy={busy()}>
          <For each={APP_ICON_PRESETS}>{preset=><button type="button" class="app-icon-preset"
            aria-label={preset.id==='fold'?'Fold (default)':preset.name}
            aria-pressed={known()&&selectedAppIcon(record())===preset.id}
            aria-disabled={busy()||loading()||!known()}
            onClick={()=>void change(preset.id)}>
            <img src={`/app-icons/${preset.id}/128.png`} width="64" height="64" alt="" draggable={false}/>
            <span>{preset.name}</span><small>{preset.id==='fold'?'Default':'Preset'}</small>
          </button>}</For>
        </div>
        <p class="app-icon-selection" role="status" aria-live="polite" aria-atomic="true">
          {busy()?'Applying icon…':loading()?'Reading selection…':known()?`Selected: ${appIconLabel(record())}${record()?'':' (default)'}`:'Selection unavailable'}
        </p>
      </Show>
      <div class="custom-icon-actions">
        <button class="btn small" disabled={loading()||busy()} onClick={()=>setIconEditor({target:props.target,mode:'upload'})}>Upload an icon</button>
        <button class="btn small" disabled={loading()||busy()} onClick={()=>setIconEditor({target:props.target,mode:'generate'})}>Generate a logo</button>
        <button class="btn small ghost" disabled={loading()||busy()||(!record()&&!error())} onClick={()=>void change()}>{resetLabel()}</button>
      </div>
      <Show when={loading()&&!isApp()}><p role="status">Reading icon…</p></Show>
      <Show when={error()}><ErrorText class="custom-icon-note" text={error()}/><Show when={damaged()}><p>{resetLabel()} keeps a copy of an unreadable saved icon.</p></Show><Show when={!damaged()}><button class="btn small ghost" onClick={()=>void read()} disabled={busy()||loading()}>Try again</button></Show></Show>
      <Show when={notice()}><p role="status">{notice()}</p></Show>
    </div>
  </section>;
}
