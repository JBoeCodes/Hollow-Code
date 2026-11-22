import assert from 'node:assert/strict';
import { test } from 'node:test';
import { join } from 'node:path';
import { directEventImports, walk, ROOT } from './check-native-events.mjs';

test('the repository passes: only the adapter imports @tauri-apps/api/event', () => {
  assert.deepEqual(directEventImports(walk(join(ROOT, 'src'))), []);
});

test('a direct import elsewhere is reported, the adapter itself is not', () => {
  const files = { 'src/a.ts': "import { listen } from '@tauri-apps/api/event';", 'src/nativeEvents.ts': "import { listen } from \"@tauri-apps/api/event\";", 'src/b.ts': "import { listen } from './nativeEvents';" };
  const found = directEventImports(Object.keys(files).map(f => join('/r', f)), f => files[f.replace('/r/', '')], '/r');
  assert.deepEqual(found, ['src/a.ts']);
});
