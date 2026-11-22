/** Browser fixture only. Real upload validation is in custom_icons.rs. */
import { drawUpload, loadIconImage, ICON_SIZES, type CustomIcon, type IconOriginal } from './customIcons';
import { emitMockEvent } from './nativeEvents';
export async function customIconFixture(command:string,args:Record<string,unknown>={}) {
  if(command==='generate_logo_options')return {value:mockLogoOptions(String(args.agent))};
  if(!['read_custom_icon','prepare_custom_icon','save_custom_icon','reset_custom_icon','select_app_icon_preset'].includes(command))return null;
  const target=command==='select_app_icon_preset'?'app':String(args.target);
  const key=`mockCustomIcon:${target}`;
  const readError=localStorage.getItem(`mockCustomIconReadError:${target}`);
  if(command==='read_custom_icon'&&readError)throw readError;
  const old=JSON.parse(localStorage.getItem(key)??'null') as CustomIcon|null;
  if(command==='read_custom_icon')return {value:old&&args.includeOriginal!==true?{...old,original:{...old.original,bytes:''},editor:null,pngs:{128:old.pngs[128]}}:old};
  if(command==='prepare_custom_icon') {
    const original=args.original as IconOriginal;
    if(original.format==='icns')throw 'Use a PNG representation in this browser preview.';
    if(original.format==='svg' && /<script|<foreignObject|onload=|href=|<!ENTITY|<style/i.test(atob(original.bytes)))throw 'Use a static SVG without scripts, styles, links or external references';
    const url=`data:image/${original.format==='svg'?'svg+xml':original.format};base64,${original.bytes}`, image=await loadIconImage(url);
    return {value:{url,width:image.naturalWidth,height:image.naturalHeight}};
  }
  if((old?.revision??null)!==(args.expected??null))throw 'ICON_CONFLICT: The saved icon changed. Nothing was replaced.';
  let record:CustomIcon|null=null;
  if(command==='select_app_icon_preset') {
    const id=String(args.preset);
    if(!['fold','threshold','cut-glass','aurora','monolith'].includes(id))throw 'Unknown app icon preset';
    if(id!=='fold') {
      const pngs:Record<number,string>={};
      for(const size of ICON_SIZES) {
        const response=await fetch(`/app-icons/${id}/${size}.png`);
        if(!response.ok)throw 'Preset artwork is unavailable';
        const bytes=new Uint8Array(await response.arrayBuffer());
        let binary='';for(const byte of bytes)binary+=String.fromCharCode(byte);
        pngs[size]=btoa(binary);
      }
      record={version:1,revision:crypto.randomUUID(),preset:id,original:{format:'png',bytes:pngs[512]},editor:{mode:'upload',fit:'fit'},pngs};
    }
  }
  if(command==='save_custom_icon') {
    const source=await loadIconImage(`data:image/png;base64,${args.rendered}`);
    const pngs=Object.fromEntries(ICON_SIZES.map(size=>{const canvas=document.createElement('canvas');canvas.width=size;canvas.height=size;drawUpload(canvas,source);return [size,canvas.toDataURL('image/png').split(',')[1]];}));
    record={version:1,revision:crypto.randomUUID(),original:args.original as IconOriginal,editor:args.editor,pngs};
  }
  localStorage.setItem(key,JSON.stringify(record));emitMockEvent('custom-icon-changed',target);
  return {value:record};
}

/** Generate a logo's stub agent (overhaul H2): four small dithered SVGs, no model call. A test sets
 *  `mockLogoOptionsError` to see a failure, or `mockLogoOptionsCount` to get fewer. */
export function mockLogoOptions(agent:string) {
  const error=localStorage.getItem('mockLogoOptionsError');
  if(error)throw error;
  const count=Number(localStorage.getItem('mockLogoOptionsCount')??4);
  const palettes=[['#1f3a2e','#d9c27a'],['#2b2340','#e3a6c8'],['#102a43','#9fd3c7'],['#3d2817','#f0b65a']];
  const options=palettes.slice(0,count).map(([ink,light],n)=>{
    let dots='';
    for(let y=0;y<16;y++)for(let x=0;x<16;x++)if(((x*3+y*5+n)%4)<(x+y)/8)dots+=`<rect x="${x*32}" y="${y*32}" width="16" height="16" fill="${light}"/>`;
    return `<svg xmlns="http://www.w3.org/2000/svg" width="512" height="512" viewBox="0 0 512 512"><rect width="512" height="512" fill="${ink}"/>${dots}<circle cx="256" cy="256" r="${120+n*16}" fill="${light}" opacity="0.85"/></svg>`;
  });
  return {agent,options};
}
