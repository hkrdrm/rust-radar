// GOES satellite imagery from NASA GIBS: which image to show for a radar frame, and tile URLs.
// Loaded by the page as a plain script and by web/satellite.test.js under node.
const SATELLITE_STEP_MIN = 10; // GIBS publishes a GOES image every 10 minutes
const SATELLITE_LIVE_START_MIN = 20; // nothing newer than this is ever published yet
const SATELLITE_LOOKBACK_STEPS = 6; // search up to an hour back past gaps and publishing delay
// West of this GOES-West has the better view, east of it GOES-East (tile-granular, so they overlap a little).
const GOES_SPLIT_LON = -112.5;
const SATELLITE_STYLES = {
  infrared: { east: 'GOES-East_ABI_Band13_Clean_Infrared', west: 'GOES-West_ABI_Band13_Clean_Infrared', maxzoom: 6 },
  geocolor: { east: 'GOES-East_ABI_GeoColor', west: 'GOES-West_ABI_GeoColor', maxzoom: 7 },
};

// Candidate image times, newest first: the step at or before the radar frame (never newer than
// what could be published), then back an hour.
function satelliteSteps(frameIso, nowMs) {
  const step = SATELLITE_STEP_MIN * 60_000;
  const newestPossible = nowMs - SATELLITE_LIVE_START_MIN * 60_000;
  const wanted = frameIso ? Math.min(Date.parse(frameIso), newestPossible) : newestPossible;
  const start = Math.floor(wanted / step) * step;
  return Array.from({ length: SATELLITE_LOOKBACK_STEPS + 1 }, (_, i) =>
    new Date(start - i * step).toISOString().replace('.000Z', 'Z'));
}

// The first time (newest first) whose image exists, or null if none in the window does.
async function newestAvailable(times, isAvailable) {
  for (const time of times) {
    if (await isAvailable(time)) return time;
  }
  return null;
}

const gibsTileUrl = (layer, time, maxzoom) =>
  `https://gibs.earthdata.nasa.gov/wmts/epsg3857/best/${layer}/default/${time}/GoogleMapsCompatible_Level${maxzoom}/{z}/{y}/{x}.png`;

// Saved dropdown value; '1' is what the earlier on/off checkbox stored for "on".
function parseStoredStyle(value) {
  if (value === '1') return 'infrared';
  return Object.hasOwn(SATELLITE_STYLES, value ?? '') ? value : 'off';
}

if (typeof module !== 'undefined') {
  module.exports = { SATELLITE_STYLES, satelliteSteps, newestAvailable, gibsTileUrl, parseStoredStyle };
}
