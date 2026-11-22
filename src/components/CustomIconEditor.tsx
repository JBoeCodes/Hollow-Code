import { For, Show, createSignal, onCleanup, onMount } from 'solid-js';
import { api } from '../api';
import { iconEditor, setIconEditor, useSavedIcon } from '../store/customIcons';
import { ICON_SIZES, UPLOAD_EDITOR, createIconTickets, fileOriginal, loadIconImage, renderUpload, svgOriginal, type IconOriginal } from '../customIcons';
import { agentName, generateLogo, logoRequestId } from '../logoGenerate';
import { confirmDialog, showConfirm } from '../store/overlays';
import { errMsg } from '../utils';
import { holdFocus } from '../focusHold';
import { trackTask } from '../store/activityDock';
import { MainAgentIndicator } from './SessionIndicator';
import { ErrorText } from './ui';
import '../styles/custom-icons.css';

/** The icon dialog (overhaul H1, H2, 2026-11-12): Upload an icon (PNG, JPEG or SVG) or Generate a logo (four dithered
 *  SVG options from an installed agent), a preview, Apply, and Reset to the stock icon. It replaced the icon maker.
 *  Closing it stops a running Generate a logo call (`cancel_logo_options`, lane S 11-13). */
export default function CustomIconEditor() {
  const target=iconEditor()!;
  const [mode,setMode]=createSignal<'upload'|'generate'>(target.mode);
  const [original,setOriginal]=createSignal<IconOriginal|null>(null), [preview,setPreview]=createSignal('');
  const [expected,setExpected]=createSignal<string|null>(null), [saved,setSaved]=createSignal(false);
  const [error,setError]=createSignal(''), [ready,setReady]=createSignal(false), [busy,setBusy]=createSignal(false), [preparing,setPreparing]=createSignal(false);
  const [readFailed,setReadFailed]=createSignal(false), [conflict,setConflict]=createSignal(false), [confirming,setConfirming]=createSignal(false);
  const [request,setRequest]=createSignal(''), [generating,setGenerating]=createSignal(false), [generateError,setGenerateError]=createSignal('');
  const [options,setOptions]=createSignal<string[]>([]), [agent,setAgent]=createSignal(''), [picked,setPicked]=createSignal(-1);
  let card!:HTMLDivElement, disposed=false, backdropOrigin=false, logoRequest:string|null=null;
  const tickets=createIconTickets();
  /** Stop the running Generate a logo call, if any (closing, or a newer one). */
  const stopLogo=()=>{const id=logoRequest;logoRequest=null;if(id)api.cancelLogoOptions(id).catch(()=>{ /* it already ended */ });};
  const title=target.target==='app'?'App icon':'COO icon';
  const resetLabel=target.target==='app'?'Reset to Fold':'Reset to default';
  const damaged=()=>readFailed()&&error().startsWith('ICON_DAMAGED:');
  const locked=()=>!ready()||readFailed()||busy()||preparing()||confirming();
  /** The choices (upload, the options, Generate) stay usable while a pick is checked: disabling the
   *  fieldset then dropped the focused option, so an arrow key after Space did nothing (2026-11-15).
   *  A newer pick, upload or generation retires the pending check (`createIconTickets`). */
  const choicesLocked=()=>!ready()||readFailed()||busy()||confirming();
  const dirty=()=>!!original();
  async function close(){
    if(busy()||confirming()||confirmDialog())return;
    if(dirty()){
      setConfirming(true);
      const discard=await showConfirm({title:'Discard this icon?',body:'Your saved icon stays unchanged.',confirmLabel:'Discard',cancelLabel:'Keep it',danger:true});
      setConfirming(false);
      if(!discard||disposed)return;
    }
    tickets.close();stopLogo();setIconEditor(null);
  }
  onMount(()=>{
    const release=holdFocus(card);card.focus();
    const key=(e:KeyboardEvent)=>{if(e.key==='Escape'&&!confirmDialog()&&!confirming()){e.preventDefault();e.stopImmediatePropagation();void close();}};
    document.addEventListener('keydown',key,true);
    onCleanup(()=>{disposed=true;tickets.close();stopLogo();release();document.removeEventListener('keydown',key,true);});
    void readSaved();
  });
  async function readSaved(){
    setReady(false);setError('');
    try{
      const r=await api.readCustomIcon(target.target);if(disposed)return;
      setExpected(r?.revision??null);setSaved(!!r);setReadFailed(false);useSavedIcon(target.target,r);
    }catch(e){if(!disposed){setError(errMsg(e));setReadFailed(true);}}
    finally{if(!disposed)setReady(true);}
  }
  /** Check `o` in the backend (the same check every save makes), draw it, and preview the 512 px render. */
  async function choose(make:()=>Promise<IconOriginal>|IconOriginal){
    const id=tickets.prepare();setPreparing(true);setError('');
    try{
      const o=await make();
      const prepared=await api.prepareCustomIcon(o);
      const image=await loadIconImage(prepared.url);
      // Sized from the backend's reading, not the image element: a viewBox-only SVG has no natural size.
      const png=renderUpload(image,prepared);
      if(!disposed&&tickets.isPrepare(id)){setOriginal(o);setPreview(`data:image/png;base64,${png}`);}
    }catch(e){if(!disposed&&tickets.isPrepare(id)){setOriginal(null);setPreview('');setError(errMsg(e));}}
    finally{if(!disposed&&tickets.isPrepare(id))setPreparing(false);}
  }
  function upload(file:File|undefined){if(!file||choicesLocked())return;setPicked(-1);void choose(()=>fileOriginal(file));}
  function pick(index:number){if(choicesLocked())return;setPicked(index);void choose(()=>svgOriginal(options()[index]));}
  async function generate(){
    if(generating()||choicesLocked())return;
    // A new ticket for the prepare side too, like an upload or pick: no older result lands after this.
    const id=tickets.generate();stopLogo();const requestId=logoRequestId();logoRequest=requestId;
    setPreparing(false);setGenerating(true);setGenerateError('');setOptions([]);setPicked(-1);
    try{
      const out=await generateLogo({agentAvailable:api.agentAvailable,cfoView:api.cfoView,generateLogoOptions:api.generateLogoOptions},target.target,request(),requestId);
      if(!disposed&&tickets.isGenerate(id)){setAgent(out.agent);setOptions(out.options);}
    }catch(e){if(!disposed&&tickets.isGenerate(id))setGenerateError(errMsg(e));}
    finally{if(logoRequest===requestId)logoRequest=null;if(!disposed&&tickets.isGenerate(id))setGenerating(false);}
  }
  async function apply(){
    const o=original();
    if(locked()||!o||!preview())return;setBusy(true);setError('');
    try{
      const revision=expected();
      if(conflict()&&!await showConfirm({title:'Replace the updated icon?',body:'Apply this icon over the latest saved one. Its original will be replaced.',confirmLabel:'Replace icon',danger:true}))return;
      if(disposed)return;
      const result=await trackTask('Save icon',()=>api.saveCustomIcon(target.target,o,preview().split(',')[1],UPLOAD_EDITOR,revision));
      if(!disposed){useSavedIcon(target.target,result);setIconEditor(null);}
    }catch(e){
      if(disposed)return;
      const message=errMsg(e);
      if(message.startsWith('ICON_CONFLICT:')){setConflict(true);await readSaved();}
      else setError(message);
    }finally{if(!disposed)setBusy(false);}
  }
  async function reset(){
    if(busy()||confirming()||(!saved()&&!damaged()))return;
    setConfirming(true);
    const yes=await showConfirm({title:'Reset to the stock icon?',body:'Your saved icon and its original are deleted.',confirmLabel:resetLabel,danger:true});
    setConfirming(false);
    if(!yes||disposed)return;
    setBusy(true);setError('');
    try{await trackTask('Reset icon',()=>api.resetCustomIcon(target.target,damaged()?null:expected()));if(!disposed){useSavedIcon(target.target,null);setIconEditor(null);}}
    catch(e){if(!disposed){const message=errMsg(e);if(message.startsWith('ICON_CONFLICT:')){setConflict(true);await readSaved();}else setError(message);}}
    finally{if(!disposed)setBusy(false);}
  }
  const svgUrl=(svg:string)=>{try{return `data:image/svg+xml;base64,${svgOriginal(svg).bytes}`;}catch{return '';}};
  return <div class="rbk-backdrop" onPointerDown={e=>{backdropOrigin=e.currentTarget===e.target;}} onPointerCancel={()=>{backdropOrigin=false;}} onClick={e=>{const fromBackdrop=backdropOrigin;backdropOrigin=false;if(fromBackdrop&&e.currentTarget===e.target)void close();}}>
    <div ref={card} class="rbk-card custom-icon-editor" role="dialog" aria-modal="true" aria-labelledby="custom-icon-title" tabIndex={-1}>
      <header class="rbk-head"><div><p class="custom-icon-eyebrow">A little more you</p><h2 id="custom-icon-title">{title}</h2></div><button class="btn ghost" onClick={close} disabled={busy()}>Cancel</button></header>
      <div class="custom-icon-scroll">
        <p class="custom-icon-note">{target.target==='app'?'Shows in Hollow, the menu bar and the Dock while Hollow is open. Finder and macOS notifications keep the stock icon.':'One icon for the COO. Its activity and attention badges stay visible.'}</p>
        <div class="custom-icon-layout">
          <div class="custom-icon-controls">
            <div class="custom-icon-modes" role="group" aria-label="Get an icon">
              <button class="btn" aria-pressed={mode()==='upload'} disabled={locked()} onClick={()=>setMode('upload')}>Upload an icon</button>
              <button class="btn" aria-pressed={mode()==='generate'} disabled={locked()} onClick={()=>setMode('generate')}>Generate a logo</button>
            </div>
            <fieldset disabled={choicesLocked()}><legend class="sr-only">{mode()==='upload'?'Upload an icon':'Generate a logo'}</legend>
            <Show when={mode()==='upload'} fallback={<>
              <label>Describe the logo (optional)<textarea rows="3" maxlength={400} value={request()} placeholder="A lantern in a doorway, deep green" onInput={e=>setRequest(e.currentTarget.value)}/></label>
              <p class="custom-icon-note">An installed agent draws four dithered SVG options: Codex when it has room, else Claude, else another agent. It uses your plan's usage.</p>
              <div><button class="btn small" disabled={generating()} onClick={()=>void generate()}>{generating()?'Drawing…':options().length?'Draw four more':'Generate four options'}</button></div>
              <Show when={generating()}><p role="status">Asking an agent for four options. This can take a minute.</p></Show>
              <Show when={generateError()}><ErrorText class="custom-icon-note" text={generateError()}/></Show>
              <Show when={options().length}>
                <section class="custom-icon-options" aria-label="Logo options">
                  <p class="custom-icon-note">Drawn by {agentName(agent())}. Pick one to preview it.</p>
                  {/* Native radios: arrow keys move and pick, one tab stop (lane S review of #1314). */}
                  <div role="radiogroup" aria-label="Logo options">
                    <For each={options()}>{(svg,i)=><label class="custom-icon-option"><input type="radio" class="sr-only" name="custom-icon-logo" aria-label={`Option ${i()+1}`} checked={picked()===i()} onChange={()=>pick(i())}/><img src={svgUrl(svg)} width="72" height="72" alt=""/></label>}</For>
                  </div>
                </section>
              </Show>
            </>}>
              <div class="custom-icon-upload"><strong>Upload an icon</strong><p>PNG, JPEG or SVG, up to 8 MiB, 4096 px per side. Static SVG only: no scripts, styles or links. It is fitted whole inside a square.</p><input type="file" accept=".png,.jpg,.jpeg,.svg,image/png,image/jpeg,image/svg+xml" aria-label="Choose icon file" onChange={e=>{upload(e.currentTarget.files?.[0]);e.currentTarget.value='';}}/></div>
              <p class="custom-icon-note">Your original is kept on this Mac, in Hollow's own app data.</p>
            </Show>
            </fieldset>
          </div>
          <div class="custom-icon-preview" aria-label="Icon preview">
            <div class="custom-icon-hero"><Show when={preview()} fallback={<p>{mode()==='upload'?'Choose an image to preview it here.':'Pick an option to preview it here.'}</p>}>{url=><img src={url()} width="192" height="192" alt={`${title} preview`}/>}</Show></div>
            <div class="custom-icon-size-strip"><For each={ICON_SIZES.slice(0,4)}>{size=><div><Show when={preview()}><img src={preview()} width={size} height={size} alt={`${size} pixel preview`}/></Show><span>{size} px</span></div>}</For></div>
            <Show when={target.target==='mainAgent'}><div class="custom-icon-badges" aria-label="COO status previews"><For each={['working','waiting','paused'] as const}>{state=><div><MainAgentIndicator state={state} waitingCount={state==='waiting'?0:undefined} size={40} image={preview()||null}/><span>{state==='working'?'Busy':state==='waiting'?'Needs you':'Paused'}</span></div>}</For><div class="custom-icon-running"><MainAgentIndicator state="working" size={40} image={preview()||null}/><b>3</b><span>3 chats</span></div></div></Show>
          </div>
        </div>
        <Show when={!ready()||preparing()}><p role="status">{preparing()?'Checking your image…':'Reading saved icon…'}</p></Show>
        <Show when={error()}><ErrorText class="custom-icon-note" text={error()}/></Show>
        <Show when={damaged()}><p class="custom-icon-note">{resetLabel} keeps a copy of the unreadable saved icon. Apply is unavailable until it can be read.</p></Show>
        <Show when={readFailed()&&!damaged()}><p class="custom-icon-note">Apply is unavailable until the saved icon can be read.</p><button class="btn small" disabled={!ready()||busy()} onClick={()=>void readSaved()}>Try again</button></Show>
        <Show when={conflict()&&!readFailed()}><p role="status" class="custom-icon-note">The saved icon changed. Your choice is kept. Apply again to review replacing the updated icon.</p></Show>
      </div>
      <footer class="rbk-foot">
        <button class="btn ghost small" disabled={busy()||confirming()||!ready()||(!saved()&&!damaged())} onClick={()=>void reset()}>{resetLabel}</button>
        <button class="btn primary" disabled={locked()||!original()||!preview()} onClick={()=>void apply()}>{busy()?'Applying…':'Apply icon'}</button>
      </footer>
    </div>
  </div>;
}
