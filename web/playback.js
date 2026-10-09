// Playback speed: how often to advance and how many radar frames to jump each time.
// Loaded by the page as a plain script and by web/playback.test.js under node.
// Uncached frames take the server ~0.25 s to draw, so ticks never go faster than 250 ms;
// higher speeds skip frames, which the server then never has to draw.
const PLAYBACK_SPEEDS = {
  1: { tickMs: 1000, step: 1 },
  2: { tickMs: 500, step: 1 },
  4: { tickMs: 250, step: 1 },
  10: { tickMs: 250, step: 3 },
  25: { tickMs: 250, step: 6 },
  100: { tickMs: 250, step: 25 },
};
const DEFAULT_PLAYBACK_SPEED = '2';

function playbackSpeed(value) {
  return PLAYBACK_SPEEDS[Object.hasOwn(PLAYBACK_SPEEDS, value) ? value : DEFAULT_PLAYBACK_SPEED];
}

// Jump ahead by step, landing on the newest frame before looping back to the oldest.
function nextFrameIndex(index, step, length) {
  const last = length - 1;
  if (index >= last) return 0;
  return Math.min(index + step, last);
}

if (typeof module !== 'undefined') {
  module.exports = { playbackSpeed, nextFrameIndex };
}
