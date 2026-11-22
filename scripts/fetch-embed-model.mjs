// Idempotent fetch + integrity-verify of the bundled semantic-search model
// (BAAI/bge-small-en-v1.5, pinned revision). The ~133MB weights are kept OUT of
// git; tauri's beforeBuildCommand runs this so a fresh clone / CI build still
// bundles the model. The pinned commit + SHA-256 check make the download
// reproducible and tamper-evident (a corrupted / MITM'd file fails the build
// loudly instead of shipping). Skips entirely — no network — when the verified
// files are already present, so repeat builds don't re-download.
//
// See docs/history/SEMANTIC_SEARCH_PLAN.md for why we bundle weights (data, not code)
// rather than commit them or fetch a runtime from a third party.
import { createHash } from 'node:crypto';
import { existsSync, readFileSync, mkdirSync, renameSync, rmSync } from 'node:fs';
import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';

const REV = '5c38ec7c405ec4b44b94cc5a9bb96e735b38267a';
const BASE = `https://huggingface.co/BAAI/bge-small-en-v1.5/resolve/${REV}`;
// candle needs only these three; verified against the pinned revision.
const FILES = {
  'config.json': '094f8e891b932f2000c92cfc663bac4c62069f5d8af5b5278c4306aef3084750',
  'tokenizer.json': 'd241a60d5e8f04cc1b2b3e9ef7a4921b27bf526d9f6050ab90f9267a1f9e5c66',
  'model.safetensors': '3c9f31665447c8911517620762200d2245a2518d6e7208acc78cd9db317e21ad',
};

const here = dirname(fileURLToPath(import.meta.url));
const dir = join(here, '..', 'src-tauri', 'models', 'bge-small-en-v1.5');

const sha256 = (path) => createHash('sha256').update(readFileSync(path)).digest('hex');

mkdirSync(dir, { recursive: true });
let fetched = 0;
for (const [name, want] of Object.entries(FILES)) {
  const dest = join(dir, name);
  if (existsSync(dest) && sha256(dest) === want) continue; // already verified
  const tmp = `${dest}.download`;
  console.log(`[fetch-embed-model] downloading ${name} …`);
  try {
    execFileSync('curl', ['-fL', '--retry', '3', `${BASE}/${name}`, '-o', tmp], {
      stdio: ['ignore', 'inherit', 'inherit'],
    });
  } catch (e) {
    rmSync(tmp, { force: true });
    console.error(`[fetch-embed-model] download failed for ${name}: ${e.message}`);
    process.exit(1);
  }
  const got = sha256(tmp);
  if (got !== want) {
    rmSync(tmp, { force: true });
    console.error(
      `[fetch-embed-model] checksum mismatch for ${name}\n  expected ${want}\n  got      ${got}`,
    );
    process.exit(1);
  }
  renameSync(tmp, dest);
  fetched++;
}
console.log(
  fetched === 0
    ? '[fetch-embed-model] model present + verified; nothing to download.'
    : `[fetch-embed-model] fetched + verified ${fetched} file(s).`,
);
