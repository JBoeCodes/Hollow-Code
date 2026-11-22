// Free local reducer benchmark; no provider, credentials or real chat data.
// Run from the checkout root: node scripts/agent-eval/transcript-bench.mjs
import { build } from 'esbuild';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { resolve, join } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { performance } from 'node:perf_hooks';

const root = fileURLToPath(new URL('../../', import.meta.url));
const scratch = await mkdtemp(join(tmpdir(), 'hollow-transcript-bench-'));
try {
  const output = resolve(scratch, 'reducer.mjs');
  await build({
    stdin: { contents: "export { createStore } from 'solid-js/store'; export { createTranscript } from './src/chatTranscript';", resolveDir: root },
    outfile: output, bundle: true, format: 'esm', platform: 'browser', conditions: ['browser'],
  });
  const { createStore, createTranscript } = await import(pathToFileURL(output));
  const samples = [];
  for (let run = 0; run < 5; run++) {
    const history = Array.from({ length: 1000 }, (_, i) => ({
      role: i % 2 ? 'assistant' : 'user', text: 'Historical answer '.repeat(50),
      tools: [{ name: 'Read', input: { file_path: `/file${i}` }, id: `old-${i}`, status: 'done' }],
    }));
    const [turns, setTurns] = createStore(history);
    const reducer = createTranscript(setTurns, () => 'benchmark', () => turns);
    let walks = 0;
    const descriptors = Object.getOwnPropertyDescriptors;
    Object.getOwnPropertyDescriptors = function (...args) { walks++; return descriptors(...args); };
    try {
      const start = performance.now();
      for (let i = 0; i < 100; i++) {
        reducer.markBoundary();
        reducer.appendToolUse('Read', { file_path: '/fixture' }, `tool-${i}`);
        reducer.applyToolResult(`tool-${i}`, 'fixture result', false);
        reducer.appendText('Done.');
      }
      samples.push({ ms: +(performance.now() - start).toFixed(2), objectWalks: walks });
    } finally { Object.getOwnPropertyDescriptors = descriptors; }
    if (turns.length !== 1100 || turns[1099].tools[0].result !== 'fixture result') throw new Error('Incorrect transcript');
  }
  console.log(JSON.stringify({ historyTurns: 1000, toolCycles: 100, samples }, null, 2));
} finally { await rm(scratch, { recursive: true, force: true }); }
