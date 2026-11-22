import { readFileSync } from 'node:fs';
import { createContext, runInContext } from 'node:vm';
import { test } from 'node:test';
import assert from 'node:assert/strict';

test('only a well-typed parent policy permits texture, without writing saved answers', () => {
  const listeners = [], events = [], sent = [];
  const parent = { postMessage: m => sent.push(m) };
  const context = createContext({
    document: { readyState: 'loading', documentElement: { setAttribute() {}, removeAttribute() {} } },
    addEventListener: (name, cb) => { if (name === 'message') listeners.push(cb); },
    dispatchEvent: e => events.push(e.type), Event: class { constructor(type) { this.type = type; } },
    setTimeout() {}, clearTimeout() {}, parent,
  });
  context.window = context;
  runInContext(readFileSync(new URL('../src/components/sandbox-doc.js', import.meta.url), 'utf8'), context);
  const send = (data, source = parent) => listeners.forEach(cb => cb({ data, source }));
  const policy = { __hc: 1, kind: 'motion', reducedMotion: false, textureAllowed: true };
  assert.equal(context.hollowCanvas.textureAllowed, false);
  send(policy, {});
  assert.equal(context.hollowCanvas.textureAllowed, false);
  send({ ...policy, textureAllowed: 'true' });
  assert.equal(context.hollowCanvas.textureAllowed, false);
  send(policy);
  assert.equal(context.hollowCanvas.textureAllowed, true);
  assert.equal(Reflect.set(context.hollowCanvas, 'textureAllowed', false), false);
  send({ ...policy, reducedMotion: true });
  assert.equal(context.hollowCanvas.textureAllowed, false);
  assert.deepEqual(events, ['hollow-texture', 'hollow-texture']);
  send({ ...policy, textureLevel: 'quiet' });
  assert.equal(context.hollowCanvas.textureLevel, 'quiet');
  assert.equal(Reflect.set(context.hollowCanvas, 'textureLevel', 'full'), false);
  send(policy);
  assert.equal(context.hollowCanvas.textureLevel, 'full');
  assert.deepEqual(sent, []);
});

test('the host runtime removes WebRTC before artifact code and prevents restoring it', () => {
  const original = class PeerConnection {};
  const context = createContext({
    RTCPeerConnection: original, webkitRTCPeerConnection: original,
    document: { readyState: 'loading' },
    addEventListener() {}, setTimeout() {}, clearTimeout() {},
  });
  context.window = context;
  runInContext(readFileSync(new URL('../src/components/sandbox-doc.js', import.meta.url), 'utf8'), context);
  for (const key of ['RTCPeerConnection', 'webkitRTCPeerConnection']) {
    assert.equal(context[key], undefined, key);
    assert.equal(Reflect.set(context, key, original), false, key);
    assert.equal(Object.getOwnPropertyDescriptor(context, key)?.configurable, false, key);
  }
});
