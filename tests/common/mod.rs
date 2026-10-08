#![allow(dead_code)]

use rust_radar::grid::{decode_dbz, Grid};

pub fn mrms_fixture() -> Vec<u8> {
    std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/mrms.grib2.gz"))
        .expect("tests/fixtures/mrms.grib2.gz missing - see Task 3 Step 1")
}

/// Decodes a PNG into raw RGBA bytes.
pub fn decode_png(png_bytes: &[u8]) -> Vec<u8> {
    let decoder = png::Decoder::new(png_bytes);
    let mut reader = decoder.read_info().unwrap();
    let mut buf = vec![0; reader.output_buffer_size()];
    reader.next_frame(&mut buf).unwrap();
    buf
}

pub fn opaque_pixels(rgba: &[u8]) -> usize {
    rgba.chunks(4).filter(|p| p[3] > 0).count()
}

/// Index of the first cell with at least 30 dBZ in the grid.
pub fn rainy_cell(grid: &Grid) -> usize {
    grid.values
        .iter()
        .position(|&v| decode_dbz(v).is_some_and(|d| d >= 30.0))
        .expect("fixture has no echo >= 30 dBZ; re-download it when it is raining somewhere")
}

/// A tile at zoom `z` that contains at least one cell with 30+ dBZ.
pub fn rainy_tile(grid: &Grid, z: u8) -> (u32, u32) {
    let (lat, lon) = grid.cell_center(rainy_cell(grid));
    rust_radar::tiles::latlon_to_tile(z, lat, lon)
}

pub fn isaias() -> rust_radar::sources::nhc::StormInfo {
    use chrono::TimeZone;
    rust_radar::sources::nhc::StormInfo {
        id: "al092026".into(),
        bin_number: "AT4".into(),
        name: "Isaias".into(),
        classification: "HU".into(),
        intensity_kt: 75,
        pressure_mb: 975,
        lat: 23.7,
        lon: -90.2,
        movement_dir: 60,
        movement_speed_mph: 10,
        advisory_num: "008".into(),
        issuance: chrono::Utc.with_ymd_and_hms(2026, 10, 8, 15, 0, 0).unwrap(),
        public_advisory_url: Some("https://www.nhc.noaa.gov/text/MIATCPAT4.shtml".into()),
        forecast_zip_url: Some("https://www.nhc.noaa.gov/gis/forecast/archive/al092026_5day_008.zip".into()),
    }
}
