// Run with: node --test 'web/*.test.js'
const test = require('node:test');
const assert = require('node:assert/strict');
const { playbackSpeed, nextFrameIndex } = require('./playback.js');

test('slow speeds step one frame at a time, faster by shortening the tick', () => {
  assert.deepEqual(playbackSpeed('1'), { tickMs: 1000, step: 1 });
  assert.deepEqual(playbackSpeed('2'), { tickMs: 500, step: 1 });
  assert.deepEqual(playbackSpeed('4'), { tickMs: 250, step: 1 });
});

test('fast speeds keep the 4× tick and skip frames instead', () => {
  assert.deepEqual(playbackSpeed('10'), { tickMs: 250, step: 3 });
  assert.deepEqual(playbackSpeed('25'), { tickMs: 250, step: 6 });
  assert.deepEqual(playbackSpeed('100'), { tickMs: 250, step: 25 });
});

test('an unknown speed falls back to 2×', () => {
  assert.deepEqual(playbackSpeed('nonsense'), { tickMs: 500, step: 1 });
  assert.deepEqual(playbackSpeed('toString'), { tickMs: 500, step: 1 });
});

test('steps forward by the skip', () => {
  assert.equal(nextFrameIndex(0, 25, 100), 25);
  assert.equal(nextFrameIndex(10, 1, 100), 11);
});

test('a skip past the end lands on the newest frame before looping', () => {
  assert.equal(nextFrameIndex(90, 25, 100), 99);
});

test('from the newest frame the loop starts over at the oldest', () => {
  assert.equal(nextFrameIndex(99, 25, 100), 0);
  assert.equal(nextFrameIndex(99, 1, 100), 0);
});
