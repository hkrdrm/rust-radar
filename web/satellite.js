// GOES infrared imagery from NASA GIBS: picks the image time for a radar frame and builds tile URLs.
// Loaded by the page as a plain script and by web/satellite.test.js under node.
const SATELLITE_STEP_MIN = 10; // GIBS publishes a GOES image every 10 minutes
const SATELLITE_LAG_MIN = 40; // newest images take ~30 minutes to appear on GIBS
const GOES_EAST_LAYER = 'GOES-East_ABI_Band13_Clean_Infrared';
const GOES_WEST_LAYER = 'GOES-West_ABI_Band13_Clean_Infrared';
// West of this GOES-West has the better view, east of it GOES-East (tile-granular, so they overlap a little).
const GOES_SPLIT_LON = -112.5;
const GIBS_MAX_ZOOM = 6; // GoogleMapsCompatible_Level6, about 2 km per pixel

// The satellite step at or before the radar frame, but never newer than GIBS reliably has.
function satelliteTime(frameIso, nowMs) {
  const step = SATELLITE_STEP_MIN * 60_000;
  const newestPublished = nowMs - SATELLITE_LAG_MIN * 60_000;
  const wanted = frameIso ? Math.min(Date.parse(frameIso), newestPublished) : newestPublished;
  return new Date(Math.floor(wanted / step) * step).toISOString().replace('.000Z', 'Z');
}

const gibsTileUrl = (layer, time) =>
  `https://gibs.earthdata.nasa.gov/wmts/epsg3857/best/${layer}/default/${time}/GoogleMapsCompatible_Level${GIBS_MAX_ZOOM}/{z}/{y}/{x}.png`;

if (typeof module !== 'undefined') module.exports = { satelliteTime, gibsTileUrl };
