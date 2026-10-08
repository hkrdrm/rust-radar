# rust-radar — Design

Date: 2026-10-08
Status: Draft for review

## Purpose

A locally run hurricane-tracking tool. A Rust service pulls weather data from
several public NOAA sources, combines it, and serves an interactive browser map
that can be zoomed, panned and clicked.

**Use cases (in priority order):**
1. **Live tracking.** Watch a storm on a map that updates on its own as new data
   arrives.
2. **Replay.** Scrub back through the last few days of radar and storm
   advisories.

**Who uses it:** one person, on this machine, in a local browser. Hosting is out
of scope.

## Decisions

| Topic | Decision |
|---|---|
| Architecture | Rust server (tokio + axum) serving a browser map (MapLibre GL JS) |
| v1 data sources | MRMS composite reflectivity, NHC active storms (cone, track, wind radii, advisories) |
| Phase 2 | GOES satellite imagery as an extra tile layer |
| Later / out of scope for v1 | Individual NEXRAD sites (Level II/III), velocity products, alerts and notifications, rainfall totals, hosting and authentication |

## Data sources

### MRMS (Multi-Radar Multi-Sensor)
- **Product:** `MergedReflectivityQCComposite_00.50`. This is the national
  composite reflectivity: a 0.01° grid, 7000×3500 cells, covering about 20–55°N
  and 130–60°W, updated about every 2 minutes.
- **Primary source:** `https://mrms.ncep.noaa.gov/data/2D/MergedReflectivityQCComposite/`
  (newest file listing).
- **Fallback and backfill:** the AWS Open Data bucket `noaa-mrms-pds`, under
  `CONUS/MergedReflectivityQCComposite_00.50/YYYYMMDD/`.
- **Format:** gzip-compressed GRIB2 using PNG packing (data representation
  template 5.41).
- **Sentinel values:** `-999` means no coverage and `-99` means missing data.
  Both are drawn transparent.

### NHC (National Hurricane Center)
- **Active storms:** `https://www.nhc.noaa.gov/CurrentStorms.json`. This gives
  the ID, name, classification, intensity, pressure, motion and advisory links.
- **Geometry** (forecast cone, forecast track, past track, wind radii): NHC's
  public GIS map service, queried with `f=geojson`. Layer IDs are looked up by
  layer name when the app starts, never hard-coded. The per-advisory GIS zip
  files are the fallback if the map service is down.
- **Archiving:** each advisory is snapshotted with its issue time, so replay can
  show the cone that was current at any past moment.

## Components

```
rust-radar/
  Cargo.toml
  rust-radar.toml        example config
  src/
    main.rs              CLI/config loading, starts pollers + server
    config.rs            config struct (TOML + CLI overrides)
    fetch.rs             Fetcher trait (HTTP GET -> bytes); reqwest impl + test fake
    sources/
      mrms.rs            find newest file, download, backfill from S3
      nhc.rs             CurrentStorms.json + GIS GeoJSON, advisory snapshots
    grid.rs              GRIB2 -> Grid { bounds, dims, values: Vec<f32> }
    archive.rs           on-disk frames + index, retention cleanup, decoded LRU cache
    palette.rs           dBZ -> RGBA (NWS reflectivity scale)
    tiles.rs             (z, x, y) Web Mercator tile -> PNG from a Grid
    server.rs            axum routes + static file serving
  web/
    index.html, app.js, style.css   MapLibre map, layers, time slider, storm panel
  tests/
    fixtures/            one real MRMS grib2.gz, sample NHC JSON/GeoJSON
```

Each module has one job and a small public interface:
- `grid` takes bytes and returns a `Grid`. It does no input/output.
- `tiles` takes a `Grid` and tile coordinates and returns PNG bytes. It does no
  input/output.
- `archive` owns the files on disk. It lists frame times and loads a grid by
  time.
- `sources` modules depend only on `Fetcher` and `archive`.
- `server` depends on `archive`, `tiles` and the NHC store.

## Data flow

1. **MRMS poller (every 60s).**
   1. List the newest file and compare its timestamp with the archive.
   2. If it is new, download it to a temporary path and decompress and decode it
      to check it.
   3. Atomically move it to `archive/mrms/YYYYMMDD-HHMMSS.grib2.gz`.
2. **Backfill on startup.** If the archive has gaps inside the retention window,
   fetch the missing frames from S3. This is limited to a configurable maximum
   (default: 6 hours).
3. **NHC poller (every 5 minutes).**
   1. Fetch `CurrentStorms.json`.
   2. For each active storm whose advisory is new, fetch the GeoJSON layers.
   3. Store them under `archive/nhc/{storm_id}/{advisory_time}/`.
4. **Retention.** An hourly task deletes MRMS frames and NHC snapshots older
   than `retention_hours` (default 72, which is about 2 GB of MRMS data).
5. **Serving.**
   1. Decoded grids are cached in an LRU cache (default capacity 10 frames).
   2. Rendered tiles are cached in memory by `(time, z, x, y)`.
   3. Each tile pixel is converted from tile coordinates to latitude/longitude
      and sampled from the grid with nearest-neighbour lookup, then colored with
      the palette.

## HTTP API

| Route | Returns |
|---|---|
| `GET /` | Frontend (static files from `web/`) |
| `GET /api/frames` | `{ "frames": ["2026-10-08T17:02:00Z", ...], "latest": "..." }` |
| `GET /tiles/{time}/{z}/{x}/{y}.png` | 256×256 radar tile; `time` is an RFC 3339 time or `latest`; 404 if the frame is unknown; transparent PNG outside the grid |
| `GET /api/storms?time=` | GeoJSON FeatureCollection of the cones, tracks and wind radii current at that time (default: now) |
| `GET /api/storms/{id}?time=` | Storm details: name, class, wind, pressure, motion, advisory text/link |
| `GET /api/status` | Last successful fetch time and age per source |

## Frontend

- MapLibre GL JS from a CDN. The basemap is OpenFreeMap's dark style (no API
  key). The style URL is configurable.
- **Radar layer:** a raster tile source pointed at `/tiles/{time}/...`, with an
  opacity slider.
- **NHC layers:** the forecast cone (semi-transparent fill), forecast and past
  track (lines plus points colored by intensity), and wind radii.
- **Click a storm** to open a side panel with its details and a link to the
  advisory.
- **Time slider:**
  - Steps through `/api/frames`.
  - Has play, pause and loop controls and a speed setting.
  - Has a "Live" button that keeps the map on the newest frame and re-checks
    for new frames every 60 seconds.
  - Changing the time updates both the radar tiles and the storm layers.
- **Status badge:** shows the age of each data source, and turns amber when
  radar is more than 10 minutes old or NHC data is more than 1 hour old.

## Error handling

- **Fetch errors:**
  - Retry with exponential backoff (capped) and log a warning.
  - Keep serving the last good data.
  - The poller never crashes the process.
- **Bad files:** any file that fails to decompress or decode is discarded and
  never enters the archive.
- **NHC layer lookup:** if the GIS map service fails, fall back to the advisory
  zip files. If both fail, show the storm points from `CurrentStorms.json`
  without cones.
- **No active storms:** the API returns an empty FeatureCollection. The panel
  says "No active tropical cyclones."
- **Outside MRMS coverage:** drawn transparent, never as "no precipitation."

## Configuration

`rust-radar.toml`, with every key overridable by a CLI flag:

```toml
port = 8080
archive_dir = "./archive"
retention_hours = 72
backfill_hours = 6
mrms_poll_secs = 60
nhc_poll_secs = 300
decoded_cache_frames = 10
basemap_style = "https://tiles.openfreemap.org/styles/dark"
```

## Testing

- **Unit tests:**
  - Palette boundaries.
  - Tile coordinate to latitude/longitude conversion, checked against known
    values.
  - Archive indexing and retention cleanup (in a temporary directory).
  - Parsing the NHC JSON and GeoJSON from fixtures.
  - Advisory-at-time lookup.
- **Decode test:** a real MRMS fixture decodes to 7000×3500 with the expected
  bounds and values in the range [-999, 90].
- **Integration test:**
  1. Start the server on a fixture archive.
  2. Check that `/api/frames` lists the fixture time.
  3. Check that a tile over a rainy area is a valid PNG with non-transparent
     pixels, and that a tile outside the grid is fully transparent.
  4. Check that `/api/storms` returns valid GeoJSON.
- **Pollers:** tested with a fake `Fetcher` returning canned responses: a new
  file, the same file again, a bad file, and an HTTP error. Tests never touch
  the network.

## Risks and open questions

- **GRIB2 PNG packing in Rust.** The plan is the `grib` crate, which should
  support template 5.41. Verify this first against a real MRMS file. Fallback:
  shell out to `wgrib2`, or use the `eccodes` bindings.
- **NHC GIS service layout** can change between seasons. This is why layer IDs
  are looked up by name, and why the advisory zip fallback exists.
- **Tile render cost.** Nearest-neighbour sampling of a 24.5M-cell grid per tile
  should take a few milliseconds. Precompute or cache further if profiling says
  so.
