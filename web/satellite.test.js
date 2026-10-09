// Run with: node --test 'web/*.test.js'
const test = require('node:test');
const assert = require('node:assert/strict');
const { satelliteTime, gibsTileUrl } = require('./satellite.js');

const at = (iso) => Date.parse(iso);
const DAY_LATER = at('2026-10-10T00:00:00Z');

test('replay rounds the radar frame down to the 10-minute satellite step', () => {
  assert.equal(satelliteTime('2026-10-08T19:47:12Z', DAY_LATER), '2026-10-08T19:40:00Z');
});

test('a frame exactly on a step keeps that step', () => {
  assert.equal(satelliteTime('2026-10-08T19:40:00Z', DAY_LATER), '2026-10-08T19:40:00Z');
});

test('live view holds back 40 minutes so the image is published', () => {
  assert.equal(satelliteTime('2026-10-08T21:42:00Z', at('2026-10-08T21:44:30Z')), '2026-10-08T21:00:00Z');
});

test('the live hold-back crosses midnight', () => {
  assert.equal(satelliteTime('2026-10-09T00:18:00Z', at('2026-10-09T00:20:00Z')), '2026-10-08T23:40:00Z');
});

test('replay rounding crosses midnight', () => {
  assert.equal(satelliteTime('2026-10-09T00:05:00Z', DAY_LATER), '2026-10-09T00:00:00Z');
});

test('no radar frame yet falls back to the live hold-back', () => {
  assert.equal(satelliteTime(undefined, at('2026-10-08T21:44:30Z')), '2026-10-08T21:00:00Z');
});

test('GIBS tile URL uses the layer, time and z/row/column order', () => {
  assert.equal(
    gibsTileUrl('GOES-East_ABI_Band13_Clean_Infrared', '2026-10-08T19:40:00Z'),
    'https://gibs.earthdata.nasa.gov/wmts/epsg3857/best/GOES-East_ABI_Band13_Clean_Infrared/default/2026-10-08T19:40:00Z/GoogleMapsCompatible_Level6/{z}/{y}/{x}.png',
  );
});
