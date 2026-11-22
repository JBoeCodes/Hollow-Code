import { createRoot, createSignal } from 'solid-js';
import { describe, expect, it } from 'vitest';
import { dirtyLabels, registerDirty } from './settingsDirty';

describe('Settings unsaved-draft registry (P1-21)', () => {
  it('lists the labels of forms with unsaved changes, and forgets a form when it unmounts', () => {
    const [edited, setEdited] = createSignal(false);
    const dispose = createRoot(dispose => {
      registerDirty('windows', edited, 'Automation windows');
      registerDirty('threshold', () => false, 'Disk space warning');
      return dispose;
    });
    expect(dirtyLabels()).toEqual([]);
    setEdited(true);
    expect(dirtyLabels()).toEqual(['Automation windows']);
    dispose();
    expect(dirtyLabels()).toEqual([]);
  });

  it('a form registered again under its id replaces the earlier entry', () => {
    const stop = registerDirty('defaults', () => true, 'Agent defaults');
    registerDirty('defaults', () => true, 'Agent defaults (new)');
    expect(dirtyLabels()).toEqual(['Agent defaults (new)']);
    // The earlier registration's cleanup no longer removes the newer one.
    stop();
    expect(dirtyLabels()).toEqual(['Agent defaults (new)']);
    registerDirty('defaults', () => false, 'Agent defaults');
    expect(dirtyLabels()).toEqual([]);
  });
});
