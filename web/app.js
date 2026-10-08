// rust-radar frontend: MRMS radar tiles + NHC storms on a MapLibre map.
const RADAR_STALE_MIN = 10;
const NHC_STALE_MIN = 60;

const $ = (id) => document.getElementById(id);
const state = { map: null, frames: [], index: -1, live: true, timer: null, stormRequest: 0, status: null, stormCount: null };

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
    // Old frames may have been pruned; keep showing the same moment if it still exists.
    state.index = Math.max(0, state.frames.indexOf(previous));
    $('slider').value = state.index;
  }
}

function showFrame(i) {
  state.index = i;
  const time = currentTime();
  state.map.getSource('radar').setTiles([tileUrl(time)]);
  $('slider').value = i;
  $('time-label').textContent = formatTime(time);
  loadStorms(time);
  renderStatus();
}

async function loadStorms(time) {
  const request = ++state.stormRequest;
  try {
    const data = await getJson(`/api/storms?time=${encodeURIComponent(time)}`);
    if (request !== state.stormRequest) return;
    state.map.getSource('storms').setData(data);
    state.stormCount = data.features.filter((f) => f.properties.layer === 'center').length;
    renderStatus();
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
  state.timer = setInterval(() => showFrame((state.index + 1) % state.frames.length), Number($('speed').value));
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

async function openPanel(stormId) {
  const time = currentTime();
  const query = time ? `?time=${encodeURIComponent(time)}` : '';
  let data;
  try {
    data = await getJson(`/api/storms/${encodeURIComponent(stormId)}${query}`);
  } catch (err) {
    console.warn(err);
    return;
  }
  const s = data.storm;
  const body = $('panel-body');
  body.replaceChildren();

  const title = document.createElement('h2');
  title.textContent = s.name;
  const subtitle = document.createElement('p');
  subtitle.className = 'subtitle';
  const strength = /^\d$/.test(data.category) ? `Category ${data.category}` : data.category;
  subtitle.textContent = `${CLASS_NAMES[s.classification] ?? s.classification} · ${strength}`;

  const rows = [
    ['Max wind', `${s.intensity_kt} kt (${Math.round(s.intensity_kt * 1.15078)} mph)`],
    ['Pressure', `${s.pressure_mb} mb`],
    ['Movement', `${compass(s.movement_dir)} at ${s.movement_speed_mph} mph`],
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
    await Promise.all([refreshFrames(), refreshStatus()]);
    setInterval(() => refreshFrames().catch(console.warn), 60_000);
    setInterval(refreshStatus, 30_000);
  });

  $('play').addEventListener('click', () => (state.timer ? stopPlayback() : startPlayback()));
  $('live').addEventListener('click', goLive);
  $('slider').addEventListener('input', (e) => {
    stopPlayback();
    setLive(false);
    showFrame(Number(e.target.value));
  });
  $('speed').addEventListener('change', () => {
    if (state.timer) { stopPlayback(); startPlayback(); }
  });
  $('opacity').addEventListener('input', (e) => {
    if (map.getLayer('radar')) map.setPaintProperty('radar', 'raster-opacity', Number(e.target.value));
  });
  $('panel-close').addEventListener('click', () => { $('panel').hidden = true; });
}

init().catch((err) => {
  $('status').textContent = `Failed to start: ${err.message}`;
  $('status').classList.add('stale');
});
