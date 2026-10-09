# rust-radar

A local hurricane tracker. It pulls NOAA MRMS composite radar (about every
2 minutes) and National Hurricane Center storm data. It keeps a rolling 72-hour
archive and serves a zoomable, clickable map with live view and replay.

## Run

```bash
cargo run --release
# open http://127.0.0.1:8080
```

On first start the app fills in the last 6 hours of radar from NOAA's AWS
bucket. That takes a few minutes and uses about 200 MB. Configuration lives in
`rust-radar.toml`, and every key also works as a CLI flag (`--help`). Set
`RUST_LOG=debug` for verbose logs.

## Data sources
- **Radar:** MRMS `MergedReflectivityQCComposite_00.50` from `mrms.ncep.noaa.gov`.
  Backfill comes from `noaa-mrms-pds` on AWS. Coverage is the continental US and
  nearby waters.
- **Storms:** `nhc.noaa.gov/CurrentStorms.json`, plus the NHC GIS map service.
  The per-advisory shapefile zip is the fallback.
- **Satellite (optional, "Satellite" menu):** GOES-East and GOES-West imagery
  from NASA GIBS. **Infrared** (ABI Band 13) colours storm-top temperature day
  and night. **GeoColor** is true colour by day and clouds with city lights by
  night. The browser loads these tiles straight from NASA, so the server does no
  extra work. Images come every 10 minutes and follow the time slider in replay.
  The page shows the newest image NASA has actually published, stepping back past
  gaps: usually 30–40 minutes behind live for infrared, about 50 for GeoColor.

## Test

```bash
cargo test
node --test 'web/*.test.js'   # frontend time logic (Node 18+)
```

Tests use the fixtures in `tests/fixtures/` and never touch the network.

## Deploy

To run it on an Ubuntu server behind nginx with a password, see
[deploy/README.md](deploy/README.md).
