import { BINDINGS, type Binding, type LiveValues } from './liveModel';

/** Detached template: parsing never loads images or runs agent code. Only named scalar
 * bindings on supported kit elements/text spans are supplied; no arbitrary paths/IPC. */
export function liveDeclarations(html: string): { keys: Binding[]; project: string; invalid: boolean } {
  const template = document.createElement('template'); template.innerHTML = html;
  const keys = new Set<Binding>(); let invalid = false;
  for (const el of template.content.querySelectorAll('[data-hollow-bind]')) {
    const key = el.getAttribute('data-hollow-bind') ?? '';
    if (!(BINDINGS as readonly string[]).includes(key) || !['HC-STAT', 'HC-PROGRESS', 'HC-GAUGE', 'SPAN'].includes(el.tagName)) invalid = true;
    else keys.add(key as Binding);
  }
  const project = template.content.querySelector('hc-canvas')?.getAttribute('data-hollow-project') ?? '';
  if (project.length > 1024 || /[\x00-\x1f]/.test(project)) invalid = true;
  return { keys: [...keys], project, invalid };
}

/** Always send only the requested finite scalars, never the raw API response. */
export function selectedValues(keys: readonly Binding[], all: LiveValues): LiveValues {
  return Object.fromEntries(keys.map(k => [k, typeof all[k] === 'number' && Number.isFinite(all[k]) ? all[k] : null]));
}
