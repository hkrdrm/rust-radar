// Run with: node --test 'web/*.test.js'
const test = require('node:test');
const assert = require('node:assert/strict');
const { SATELLITE_STYLES, satelliteSteps, newestAvailable, gibsTileUrl, parseStoredStyle } = require('./satellite.js');

const at = (iso) => Date.parse(iso);
const DAY_LATER = at('2026-10-10T00:00:00Z');

test('replay starts at the 10-minute step at or before the radar frame and looks back an hour', () => {
  assert.deepEqual(satelliteSteps('2026-10-08T19:47:12Z', DAY_LATER), [
    '2026-10-08T19:40:00Z', '2026-10-08T19:30:00Z', '2026-10-08T19:20:00Z', '2026-10-08T19:10:00Z',
    '2026-10-08T19:00:00Z', '2026-10-08T18:50:00Z', '2026-10-08T18:40:00Z',
  ]);
});

test('a frame exactly on a step starts at that step', () => {
  assert.equal(satelliteSteps('2026-10-08T19:40:00Z', DAY_LATER)[0], '2026-10-08T19:40:00Z');
});

test('live starts 20 minutes back, since newer images are never published yet', () => {
  assert.equal(satelliteSteps(undefined, at('2026-10-08T21:44:30Z'))[0], '2026-10-08T21:20:00Z');
});

test('a radar frame newer than that is held back the same way', () => {
  assert.equal(satelliteSteps('2026-10-08T21:42:00Z', at('2026-10-08T21:44:30Z'))[0], '2026-10-08T21:20:00Z');
});

test('steps cross midnight', () => {
  const steps = satelliteSteps(undefined, at('2026-10-09T00:15:00Z'));
  assert.equal(steps[0], '2026-10-08T23:50:00Z');
  assert.equal(steps[6], '2026-10-08T22:50:00Z');
});

test('newestAvailable skips a gap and stops at the first image that exists', async () => {
  const asked = [];
  const found = await newestAvailable(['20:40', '20:30', '20:20', '20:10'], async (t) => {
    asked.push(t);
    return t === '20:20' || t === '20:10';
  });
  assert.equal(found, '20:20');
  assert.deepEqual(asked, ['20:40', '20:30', '20:20']);
});

test('newestAvailable returns null when nothing in the window exists', async () => {
  assert.equal(await newestAvailable(['20:40', '20:30'], async () => false), null);
});

test('GIBS tile URL uses the layer, time, zoom level and z/row/column order', () => {
  assert.equal(
    gibsTileUrl('GOES-East_ABI_GeoColor', '2026-10-08T19:40:00Z', 7),
    'https://gibs.earthdata.nasa.gov/wmts/epsg3857/best/GOES-East_ABI_GeoColor/default/2026-10-08T19:40:00Z/GoogleMapsCompatible_Level7/{z}/{y}/{x}.png',
  );
});

test('both styles name a GOES-East and GOES-West layer and their own zoom limit', () => {
  assert.deepEqual(SATELLITE_STYLES.infrared,
    { east: 'GOES-East_ABI_Band13_Clean_Infrared', west: 'GOES-West_ABI_Band13_Clean_Infrared', maxzoom: 6 });
  assert.deepEqual(SATELLITE_STYLES.geocolor,
    { east: 'GOES-East_ABI_GeoColor', west: 'GOES-West_ABI_GeoColor', maxzoom: 7 });
});

test('the stored choice survives, and the old on/off checkbox value maps to infrared', () => {
  assert.equal(parseStoredStyle('1'), 'infrared');
  assert.equal(parseStoredStyle('0'), 'off');
  assert.equal(parseStoredStyle(null), 'off');
  assert.equal(parseStoredStyle('geocolor'), 'geocolor');
  assert.equal(parseStoredStyle('infrared'), 'infrared');
  assert.equal(parseStoredStyle('nonsense'), 'off');
  assert.equal(parseStoredStyle('toString'), 'off');
});
