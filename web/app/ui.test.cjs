const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const { dockFrames } = require('./ui.js');

const rect = (x, y, width, height) => ({ x, y, width, height });
const large = { dock: rect(260, 150, 800, 450), video: rect(260, 150, 800, 450) };
const mini = { dock: rect(944, 600, 310, 208.375), video: rect(944, 600, 310, 174.375) };
const numbers = transform => transform.match(/[-+]?(?:\d*\.)?\d+(?:e[-+]?\d+)?/gi).map(Number);
const near = (a, b) => assert.ok(Math.abs(a - b) < 0.00001, `${a} != ${b}`);

function verifyPicture(before, after) {
  const frames = dockFrames(before, after);
  for (let i = 0; i < frames.dock.length; i++) {
    const t = frames.dock[i].offset;
    const [dx, dy, sx, sy] = numbers(frames.dock[i].transform);
    const [vx, vy, vsx, vsy] = numbers(frames.video[i].transform);
    const mix = (a, b) => a + (b - a) * t;
    near(after.dock.x + dx + sx * (after.video.x - after.dock.x + vx), mix(before.video.x, after.video.x));
    near(after.dock.y + dy + sy * (after.video.y - after.dock.y + vy), mix(before.video.y, after.video.y));
    near(after.video.width * sx * vsx, mix(before.video.width, after.video.width));
    near(after.video.height * sy * vsy, mix(before.video.height, after.video.height));
    near((after.video.width * sx * vsx) / (after.video.height * sy * vsy), 16 / 9);
  }
}

test('live picture keeps its proportions and position while the frame gains mini controls', () => {
  verifyPicture(large, mini);
  verifyPicture(mini, large);
});
test('an interrupted transition can restart from the current on-screen rectangles', () => {
  const middle = { dock: rect(550, 320, 580, 330), video: rect(550, 320, 580, 326.25) };
  verifyPicture(middle, large);
  verifyPicture(middle, mini);
});
test('minimized or invalid rectangles do not produce unusable animation matrices', () => {
  assert.equal(dockFrames({ ...large, dock: rect(0, 0, 0, 0) }, mini), null);
  assert.equal(dockFrames(large, { ...mini, video: rect(NaN, 0, 10, 10) }), null);
});

function environment(reduce) {
  const listeners = {};
  const sandbox = {
    module: { exports: {} },
    document: { hidden: false, addEventListener: (name, callback) => { listeners[name] = callback; } },
    addEventListener() {},
    matchMedia: () => ({ matches: reduce, addEventListener() {} }),
    getComputedStyle: () => ({ opacity: '1', transform: 'none' }),
  };
  vm.runInNewContext(fs.readFileSync(require.resolve('./ui.js'), 'utf8'), sandbox);
  const animations = [];
  const dialog = { open: false, inert: false, style: { removeProperty() {} },
    toggleAttribute() {}, removeAttribute() {}, showModal() { this.open = true; }, close() { this.open = false; },
    animate(frames, options) { const animation = { cancel() {}, finished: new Promise(() => {}) }; animations.push({ frames, options }); return animation; } };
  const ui = sandbox.module.exports;
  ui.initialize(sandbox.document);
  return { ui, dialog, animations, listeners };
}
test('reduced motion preserves a gentle confirmation without spatial movement', () => {
  const { ui, dialog, animations } = environment(true);
  ui.openDialog(dialog);
  assert.equal(dialog.open, true);
  assert.ok(animations[0].frames.every(frame => frame.transform === 'none'));
  assert.equal(animations[0].frames[0].opacity, 0);
  assert.equal(animations[0].frames[1].opacity, 1);
});
test('keyboard opens and closes the modal immediately, including during pointer motion', () => {
  const { ui, dialog, animations, listeners } = environment(false);
  listeners.keydown();
  ui.openDialog(dialog);
  assert.equal(dialog.open, true);
  assert.equal(animations.length, 0);
  ui.closeDialog(dialog);
  assert.equal(dialog.open, false);
  listeners.pointerdown();
  ui.openDialog(dialog);
  assert.equal(animations.length, 1);
  listeners.keydown();
  ui.closeDialog(dialog);
  assert.equal(dialog.open, false);
  assert.equal(dialog.inert, false);
});
