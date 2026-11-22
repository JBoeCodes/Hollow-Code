import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { createHash } from 'node:crypto';

const file = name => readFileSync(new URL('../' + name, import.meta.url));
const fold = size => file(`public/app-icons/fold/${size}.png`);
const pngSize = bytes => [bytes.readUInt32BE(16), bytes.readUInt32BE(20)];

test('Fold preserves the selected hand-drawn small icons across bundle and favicon', () => {
  const authored = {
    16: 'fd0154f6c2cd1b7bd08550ff700aaa754c19a8cdf97826b6891be6519d686cfa',
    32: '0640e51377a6e884f69dbe76cb6448dabac18888519203c7ff766f1f7424b9b4',
  };
  for (const [size, hash] of Object.entries(authored)) {
    assert.equal(createHash('sha256').update(fold(size)).digest('hex'), hash);
    assert.deepEqual(file(`src-tauri/icons/${size}x${size}.png`), fold(size));
    assert.deepEqual(file(`public/hollow-${size}.png`), fold(size));
  }
  for (const [size, name] of [[64,'64x64.png'],[128,'128x128.png'],[256,'128x128@2x.png'],[512,'icon.png'],[1024,'source.png']]) {
    assert.deepEqual(file('src-tauri/icons/' + name), fold(size));
    assert.deepEqual(pngSize(fold(size)), [size, size]);
  }
  assert.deepEqual(file('public/hollow.svg'), file('public/app-icons/fold/source.svg'));
  assert.deepEqual(pngSize(file('src/assets/brand/hollow-mark-96.png')), [96,96]);
  assert.deepEqual(file('src/assets/brand/hollow-mark-96.png'),fold(96));
  assert.equal(createHash('sha256').update(fold(96)).digest('hex'),
    '4a766c1bb829ddd04c757121fe267bac9b66ef04c131c3cb80088a8295ae666b');
  assert.equal(createHash('sha256').update(file('src/assets/brand/main-agent-fold-512.png')).digest('hex'),
    'fbf41e4f7c13d4112758265734231d42b1ca7b808e72a00471de9485ceb56786');
});

test('every ICNS representation contains the selected PNG verbatim', () => {
  const bytes = file('src-tauri/icons/icon.icns');
  assert.equal(bytes.subarray(0,4).toString(), 'icns');
  assert.equal(bytes.readUInt32BE(4), bytes.length);
  const expected = {icp4:16,icp5:32,icp6:64,ic07:128,ic08:256,ic09:512,ic10:1024,ic11:32,ic12:64,ic13:256,ic14:512};
  const seen = new Set();
  for (let offset=8; offset<bytes.length;) {
    const tag=bytes.subarray(offset,offset+4).toString(), length=bytes.readUInt32BE(offset+4);
    assert.ok(length>8 && offset+length<=bytes.length);
    assert.ok(expected[tag] && !seen.has(tag));
    assert.deepEqual(bytes.subarray(offset+8,offset+length), fold(expected[tag]));
    seen.add(tag);offset+=length;
  }
  assert.equal(seen.size,11);
});

test('ICO keeps all five authored sizes and bounded offsets', () => {
  const bytes=file('src-tauri/icons/icon.ico');
  assert.equal(bytes.readUInt16LE(0),0);assert.equal(bytes.readUInt16LE(2),1);
  assert.equal(bytes.readUInt16LE(4),5);
  for (const [i,size] of [16,32,64,128,256].entries()) {
    const row=6+i*16, length=bytes.readUInt32LE(row+8), offset=bytes.readUInt32LE(row+12);
    assert.equal(bytes[row] || 256,size);assert.equal(bytes[row+1] || 256,size);
    assert.ok(offset>=86 && offset+length<=bytes.length);
    assert.deepEqual(bytes.subarray(offset,offset+length),fold(size));
  }
});
