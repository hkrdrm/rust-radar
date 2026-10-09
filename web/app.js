// rust-radar frontend: MRMS radar tiles + GOES satellite + NHC storms on a MapLibre map.
const RADAR_STALE_MIN = 10;
const NHC_STALE_MIN = 60;

const $ = (id) => document.getElementById(id);
const state = { map: null, frames: [], index: -1, live: true, timer: null, stormRequest: 0, status: null, stormCount: null, openStormId: null,
  satTime: null, satShown: null, satRequest: 0, satPending: false };
const SATELLITE_KEY = 'rust-radar.satellite';
const satelliteStyle = () => $('satellite').value; // 'off', 'infrared' or 'geocolor'
const satSources = (style) => [`sat-${style}-west`, `sat-${style}-east`];
const satAvailability = new Map(); // "style|time" -> Promise<boolean>

const tileUrl = (time) => `/tiles/${encodeURIComponent(time)}/{z}/{x}/{y}.png`;
const currentTime = () => state.frames[state.index];
const isLayer = (name) => ['==', ['get', 'layer'], name];

async function getJson(url) {
  const resp = await fetch(url);
  if (!resp.ok) throw new Error(`${url}: HTTP ${resp.status}`);
  return resp.json();
}

const formatTime = (iso) =>
  new Date(iso).toLocaleString([], { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit', timeZoneName: 'short' });

const CATEGORY_COLORS = ['match', ['get', 'category'],
  'TD', '#5ebaff', 'TS', '#00faf4', '1', '#ffffcc', '2', '#ffe775', '3', '#ffc140', '4', '#ff8f20', '5', '#ff6060', '#cccccc'];
const WIND_COLORS = ['step', ['coalesce', ['get', 'wind_kt'], 0],
  '#5ebaff', 34, '#00faf4', 64, '#ffffcc', 83, '#ffe775', 96, '#ffc140', 113, '#ff8f20', 137, '#ff6060'];
const CLASS_NAMES = {
  TD: 'Tropical Depression', TS: 'Tropical Storm', HU: 'Hurricane', STD: 'Subtropical Depression',
  STS: 'Subtropical Storm', PTC: 'Post-Tropical Cyclone', PC: 'Potential Tropical Cyclone',
};

function addLayers(map) {
  const layers = map.getStyle().layers;
  const firstSymbol = layers.find((l) => l.type === 'symbol')?.id;
  const font = layers.find((l) => Array.isArray(l.layout?.['text-font']))?.layout['text-font'];

  // Satellite covers the basemap's land and water but stays under its roads, borders and labels.
  // Opaque, so where the two sources' tiles overlap GOES-East simply wins instead of doubling up.
  const firstLine = layers.find((l) => l.type !== 'background' && l.type !== 'fill')?.id;
  // Hidden until showSatellite has found an image that exists.
  const placeholderTime = satelliteSteps(undefined, Date.now())[0];
  for (const [style, { east, west, maxzoom }] of Object.entries(SATELLITE_STYLES)) {
    const [westId, eastId] = satSources(style);
    for (const [id, layer, bounds] of [
      [westId, west, [-180, -85, GOES_SPLIT_LON, 85]],
      [eastId, east, [GOES_SPLIT_LON, -85, 180, 85]],
    ]) {
      map.addSource(id, { type: 'raster', tiles: [gibsTileUrl(layer, placeholderTime, maxzoom)], tileSize: 256, maxzoom, bounds });
      map.addLayer({ id, type: 'raster', source: id, layout: { visibility: 'none' },
        paint: { 'raster-fade-duration': 0 } }, firstLine);
    }
  }

  map.addSource('radar', { type: 'raster', tiles: [tileUrl('latest')], tileSize: 256, maxzoom: 10 });
  map.addLayer({ id: 'radar', type: 'raster', source: 'radar',
    paint: { 'raster-opacity': Number($('opacity').value), 'raster-fade-duration': 0 } }, firstSymbol);

  map.addSource('storms', { type: 'geojson', data: { type: 'FeatureCollection', features: [] } });
  map.addLayer({ id: 'storm-cone', type: 'fill', source: 'storms', filter: isLayer('cone'),
    paint: { 'fill-color': '#ffffff', 'fill-opacity': 0.12 } });
  map.addLayer({ id: 'storm-cone-outline', type: 'line', source: 'storms', filter: isLayer('cone'),
    paint: { 'line-color': '#ffffff', 'line-width': 1.5 } });
  map.addLayer({ id: 'storm-wind-radii', type: 'line', source: 'storms', filter: isLayer('wind_radii'),
    paint: { 'line-color': ['match', ['get', 'radii'], 34, '#ffd54f', 50, '#ff9800', 64, '#f44336', '#ffffff'], 'line-width': 1 } });
  map.addLayer({ id: 'storm-past-track', type: 'line', source: 'storms', filter: isLayer('past_track'),
    paint: { 'line-color': '#bbbbbb', 'line-width': 2 } });
  map.addLayer({ id: 'storm-forecast-track', type: 'line', source: 'storms', filter: isLayer('forecast_track'),
    paint: { 'line-color': '#ffffff', 'line-width': 2, 'line-dasharray': [2, 2] } });
  map.addLayer({ id: 'storm-points', type: 'circle', source: 'storms',
    filter: ['any', isLayer('forecast_points'), isLayer('past_points')],
    paint: { 'circle-radius': 4, 'circle-color': WIND_COLORS, 'circle-stroke-color': '#000', 'circle-stroke-width': 1 } });
  map.addLayer({ id: 'storm-center', type: 'circle', source: 'storms', filter: isLayer('center'),
    paint: { 'circle-radius': 9, 'circle-color': CATEGORY_COLORS, 'circle-stroke-color': '#000', 'circle-stroke-width': 2 } });
  map.addLayer({ id: 'storm-label', type: 'symbol', source: 'storms', filter: isLayer('center'),
    layout: { 'text-field': ['get', 'name'], 'text-offset': [0, 1.4], 'text-size': 13, ...(font ? { 'text-font': font } : {}) },
    paint: { 'text-color': '#ffffff', 'text-halo-color': '#000000', 'text-halo-width': 1.5 } });
}

async function refreshFrames() {
  const previous = currentTime();
  const data = await getJson('/api/frames');
  state.frames = data.frames;
  $('slider').max = Math.max(0, state.frames.length - 1);
  if (state.frames.length === 0) {
    $('time-label').textContent = 'Waiting for the first radar frame…';
    return;
  }
  if (state.live || !previous) {
    showFrame(state.frames.length - 1);
  } else {
    // Old frames may have been pruned; keep showing the same moment if it still exists,
    // otherwise move to the oldest frame that does.
    const kept = state.frames.indexOf(previous);
    if (kept >= 0) {
      state.index = kept;
      $('slider').value = kept;
    } else {
      showFrame(0);
    }
  }
}

function showFrame(i) {
  state.index = i;
  const time = currentTime();
  state.map.getSource('radar').setTiles([tileUrl(time)]);
  $('slider').value = i;
  renderTimeLabel();
  showSatellite();
  // Live storms come from "now", independent of radar, so a radar outage cannot hide or freeze them.
  loadStorms(state.live ? null : time);
  renderStatus();
}

function renderTimeLabel() {
  const frame = currentTime();
  if (!frame) return; // keep the "waiting for radar" message
  let text = formatTime(frame);
  if (satelliteStyle() !== 'off') {
    const sat = state.satTime && new Date(state.satTime).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
    text += sat ? ` · Sat ${sat}` : state.satPending ? ' · Sat …' : ' · Sat unavailable';
  }
  $('time-label').textContent = text;
}

// Whether GIBS has published this image for both satellites, checked with one small HEAD request each.
function imageExists(style, time) {
  const key = `${style}|${time}`;
  if (!satAvailability.has(key)) {
    const { east, west, maxzoom } = SATELLITE_STYLES[style];
    const head = (layer) => fetch(gibsTileUrl(layer, time, maxzoom).replace('{z}/{y}/{x}', '0/0/0'), { method: 'HEAD' })
      .then((resp) => resp.ok, () => false);
    const probe = Promise.all([head(east), head(west)]).then(([e, w]) => e && w);
    satAvailability.set(key, probe);
    // A missing recent image may still be on its way; forget the miss so it is checked again.
    probe.then((ok) => { if (!ok) setTimeout(() => satAvailability.delete(key), 120_000); });
  }
  return satAvailability.get(key);
}

// Point a satellite source at a new image. MapLibre's reload skips tiles that failed before (say, asked
// for while NASA was still publishing), which would leave holes; retry those too. Internal API, 4.7.1.
function setSatelliteTiles(id, url) {
  state.map.getSource(id).setTiles([url]);
  const cache = state.map.style.sourceCaches[id];
  for (const key in cache._tiles) {
    if (cache._tiles[key].state === 'errored') cache._reloadTile(key, 'reloading');
  }
}

// Show the selected satellite style at the newest published image for the current frame.
async function showSatellite() {
  if (!state.map?.getLayer('sat-infrared-east')) return; // map not loaded yet; it calls back in
  const style = satelliteStyle();
  const request = ++state.satRequest;
  state.satPending = style !== 'off';
  if (state.satPending) {
    const steps = satelliteSteps(state.live ? undefined : currentTime(), Date.now());
    const time = await newestAvailable(steps, (t) => imageExists(style, t));
    if (request !== state.satRequest) return; // a newer frame or style took over
    state.satPending = false;
    state.satTime = time; // null: nothing published in the last hour; keep the previous image up
    if (time && `${style}|${time}` !== state.satShown) {
      state.satShown = `${style}|${time}`;
      const { east, west, maxzoom } = SATELLITE_STYLES[style];
      const [westId, eastId] = satSources(style);
      setSatelliteTiles(westId, gibsTileUrl(west, time, maxzoom));
      setSatelliteTiles(eastId, gibsTileUrl(east, time, maxzoom));
    }
  }
  // A style becomes visible only once its sources point at an image that exists.
  for (const s of Object.keys(SATELLITE_STYLES)) {
    const on = s === style && state.satShown?.startsWith(`${s}|`);
    for (const id of satSources(s)) state.map.setLayoutProperty(id, 'visibility', on ? 'visible' : 'none');
  }
  renderTimeLabel();
}

function setSatelliteStyle(style) {
  try { localStorage.setItem(SATELLITE_KEY, style); } catch (err) { /* storage unavailable */ }
  state.satTime = null;
  showSatellite();
}

async function loadStorms(time) {
  const request = ++state.stormRequest;
  try {
    const data = await getJson(time ? `/api/storms?time=${encodeURIComponent(time)}` : '/api/storms');
    if (request !== state.stormRequest) return;
    state.map.getSource('storms').setData(data);
    state.stormCount = data.features.filter((f) => f.properties.layer === 'center').length;
    renderStatus();
    if (state.openStormId) openPanel(state.openStormId);
  } catch (err) {
    console.warn(err);
  }
}

function setLive(live) {
  state.live = live;
  $('live').classList.toggle('active', live);
}

function stopPlayback() {
  clearInterval(state.timer);
  state.timer = null;
  $('play').textContent = '▶';
}

function startPlayback() {
  if (state.frames.length < 2) return;
  setLive(false);
  const { tickMs, step } = playbackSpeed($('speed').value);
  state.timer = setInterval(() => {
    // Wait for the current frame's tiles; otherwise requests pile up faster than the server can render.
    const style = satelliteStyle();
    const sources = style === 'off' ? ['radar'] : ['radar', ...satSources(style)];
    if (state.satPending || !sources.every((id) => state.map.isSourceLoaded(id))) return;
    showFrame(nextFrameIndex(state.index, step, state.frames.length));
  }, tickMs);
  $('play').textContent = '⏸';
}

function goLive() {
  stopPlayback();
  setLive(true);
  if (state.frames.length) showFrame(state.frames.length - 1);
}

async function refreshStatus() {
  try {
    state.status = await getJson('/api/status');
  } catch (err) {
    state.status = null;
  }
  renderStatus();
}

const ageMinutes = (iso) => (iso ? (Date.now() - new Date(iso).getTime()) / 60000 : Infinity);

function describeAge(minutes) {
  if (!Number.isFinite(minutes)) return 'none';
  if (minutes < 1) return '<1m';
  if (minutes < 120) return `${Math.round(minutes)}m`;
  return `${Math.round(minutes / 60)}h`;
}

function renderStatus() {
  const sources = state.status?.sources ?? {};
  const radarAge = ageMinutes(state.frames[state.frames.length - 1]);
  const nhcAge = ageMinutes(sources.nhc?.last_success);
  const badge = $('status');
  badge.textContent = `Radar ${describeAge(radarAge)} · NHC ${describeAge(nhcAge)}`;
  if (state.stormCount === 0) badge.textContent += ' · No active tropical cyclones';
  badge.classList.toggle('stale', radarAge > RADAR_STALE_MIN || nhcAge > NHC_STALE_MIN);
  badge.title = [sources.mrms?.last_error, sources.nhc?.last_error].filter(Boolean).join('\n');
}

function compass(degrees) {
  const points = ['N', 'NNE', 'NE', 'ENE', 'E', 'ESE', 'SE', 'SSE', 'S', 'SSW', 'SW', 'WSW', 'W', 'WNW', 'NW', 'NNW'];
  return points[Math.round((degrees % 360) / 22.5) % 16];
}

function closePanel() {
  state.openStormId = null;
  $('panel').hidden = true;
}

function describeMovement(dir, speed) {
  if (dir == null || speed == null) return 'Unknown';
  if (speed === 0) return 'Stationary';
  return `${compass(dir)} at ${speed} mph`;
}

async function openPanel(stormId) {
  state.openStormId = stormId;
  // Same moment as the storm layer: "now" when live, the frame time when replaying.
  const time = state.live ? null : currentTime();
  const query = time ? `?time=${encodeURIComponent(time)}` : '';
  const resp = await fetch(`/api/storms/${encodeURIComponent(stormId)}${query}`).catch((err) => err);
  if (state.openStormId !== stormId) return;
  if (resp instanceof Response && resp.status === 404) {
    closePanel(); // no advisory for this storm at this moment (ended, or before it formed)
    return;
  }
  if (!(resp instanceof Response) || !resp.ok) {
    console.warn(resp);
    return;
  }
  const data = await resp.json();
  const s = data.storm;
  const body = $('panel-body');
  body.replaceChildren();

  const title = document.createElement('h2');
  title.textContent = s.name;
  const subtitle = document.createElement('p');
  subtitle.className = 'subtitle';
  const className = CLASS_NAMES[s.classification] ?? s.classification;
  subtitle.textContent = /^\d$/.test(data.category) ? `${className} · Category ${data.category}` : className;

  const rows = [
    ['Max wind', `${s.intensity_kt} kt (${Math.round(s.intensity_kt * 1.15078)} mph)`],
    ['Pressure', `${s.pressure_mb} mb`],
    ['Movement', describeMovement(s.movement_dir, s.movement_speed_mph)],
    ['Position', `${s.lat.toFixed(1)}°, ${s.lon.toFixed(1)}°`],
    ['Advisory', `#${s.advisory_num || '?'} · ${formatTime(s.issuance)}`],
  ];
  if (data.geometry_source === 'none') rows.push(['Forecast cone', 'unavailable']);
  const list = document.createElement('dl');
  for (const [label, value] of rows) {
    const dt = document.createElement('dt');
    dt.textContent = label;
    const dd = document.createElement('dd');
    dd.textContent = value;
    list.append(dt, dd);
  }
  body.append(title, subtitle, list);

  if (s.public_advisory_url) {
    const link = document.createElement('a');
    link.href = s.public_advisory_url;
    link.target = '_blank';
    link.rel = 'noopener';
    link.textContent = 'Read the public advisory ↗';
    body.append(link);
  }
  $('panel').hidden = false;
}

async function init() {
  try { $('satellite').value = parseStoredStyle(localStorage.getItem(SATELLITE_KEY)); } catch (err) { /* storage unavailable */ }
  const config = await getJson('/api/config');
  const map = new maplibregl.Map({ container: 'map', style: config.basemap_style, center: [-85, 27], zoom: 4 });
  state.map = map;
  map.addControl(new maplibregl.NavigationControl(), 'top-right');
  // Switching frames cancels in-flight tile requests; MapLibre reports those as errors.
  map.on('error', (e) => {
    const aborted = e.error?.name === 'AbortError' || e.error?.message === 'AbortError';
    if (!aborted) console.error(e.error);
  });

  map.on('load', async () => {
    addLayers(map);
    for (const layer of ['storm-center', 'storm-cone']) {
      map.on('click', layer, (e) => openPanel(e.features[0].properties.storm_id));
      map.on('mouseenter', layer, () => { map.getCanvas().style.cursor = 'pointer'; });
      map.on('mouseleave', layer, () => { map.getCanvas().style.cursor = ''; });
    }
    // Register polling first so a failed first fetch is retried rather than fatal.
    setInterval(() => refreshFrames().catch(console.warn), 60_000);
    setInterval(() => { if (state.live) loadStorms(null); }, 60_000);
    setInterval(refreshStatus, 30_000);
    loadStorms(null);
    refreshStatus();
    refreshFrames().catch((err) => {
      console.warn(err);
      $('time-label').textContent = 'Radar unavailable — retrying…';
    });
  });

  $('play').addEventListener('click', () => (state.timer ? stopPlayback() : startPlayback()));
  $('live').addEventListener('click', goLive);
  // While dragging, only move the label; load the frame once the user pauses or releases.
  let scrub = null;
  $('slider').addEventListener('input', (e) => {
    stopPlayback();
    setLive(false);
    const i = Number(e.target.value);
    $('time-label').textContent = formatTime(state.frames[i]);
    clearTimeout(scrub);
    scrub = setTimeout(() => showFrame(i), 250);
  });
  $('speed').addEventListener('change', () => {
    if (state.timer) { stopPlayback(); startPlayback(); }
  });
  $('opacity').addEventListener('input', (e) => {
    if (map.getLayer('radar')) map.setPaintProperty('radar', 'raster-opacity', Number(e.target.value));
  });
  $('satellite').addEventListener('change', (e) => setSatelliteStyle(e.target.value));
  $('panel-close').addEventListener('click', closePanel);
}

init().catch((err) => {
  $('status').textContent = `Failed to start: ${err.message}`;
  $('status').classList.add('stale');
});
