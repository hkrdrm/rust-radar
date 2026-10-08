//! Decoding MRMS GRIB2 files into a compact lat/lon grid of reflectivity.

use std::io::Read;

use anyhow::{anyhow, bail, Context};
use grib::LatLons;

/// Packed value for clear air: radar sees nothing (-99) or below 0 dBZ.
pub const NO_DATA: u8 = 0;
/// Packed value for "beyond radar coverage" (MRMS -999): unknown, not clear.
pub const NO_COVERAGE: u8 = u8::MAX;
/// MRMS writes -999 outside radar coverage; anything this low is that sentinel.
const NO_COVERAGE_BELOW: f32 = -500.0;

/// Packs dBZ into one byte: 0.5 dBZ steps from 0 to 126.5 dBZ.
pub fn encode_dbz(dbz: f32) -> u8 {
    if dbz < NO_COVERAGE_BELOW {
        return NO_COVERAGE;
    }
    if !dbz.is_finite() || dbz < 0.0 {
        return NO_DATA;
    }
    ((dbz * 2.0).round() + 1.0).min(f32::from(NO_COVERAGE - 1)) as u8
}

pub fn decode_dbz(value: u8) -> Option<f32> {
    match value {
        NO_DATA | NO_COVERAGE => None,
        v => Some(f32::from(v - 1) / 2.0),
    }
}

/// Ordering for pooling: any echo beats clear air, which beats no coverage.
fn strength(value: u8) -> u16 {
    match value {
        NO_COVERAGE => 0,
        v => u16::from(v) + 1,
    }
}

/// A regular lat/lon grid. Row 0 is the northernmost row; column 0 the westernmost.
#[derive(Debug, Clone)]
pub struct Grid {
    pub nx: usize,
    pub ny: usize,
    /// Latitude of the centre of row 0, degrees.
    pub lat0: f64,
    /// Longitude of the centre of column 0, degrees (-180..180).
    pub lon0: f64,
    /// Row spacing in degrees (positive; rows go south).
    pub dlat: f64,
    /// Column spacing in degrees.
    pub dlon: f64,
    /// Row-major `encode_dbz` values, `nx * ny` long.
    pub values: Vec<u8>,
}

impl Grid {
    pub fn from_grib2_gz(gz: &[u8]) -> anyhow::Result<Grid> {
        let mut raw = Vec::new();
        flate2::read::GzDecoder::new(gz)
            .read_to_end(&mut raw)
            .context("decompressing gzip")?;
        Grid::from_grib2(&raw)
    }

    pub fn from_grib2(raw: &[u8]) -> anyhow::Result<Grid> {
        let grib2 = grib::from_bytes(raw).map_err(|e| anyhow!("parsing GRIB2: {e}"))?;
        let (_, sub) = grib2.iter().next().context("GRIB2 file has no submessages")?;
        let (nx, ny) = sub.grid_shape().map_err(|e| anyhow!("reading grid shape: {e}"))?;
        if nx < 2 || ny < 2 {
            bail!("grid too small: {nx}x{ny}");
        }
        let mut points = sub.latlons().map_err(|e| anyhow!("computing lat/lons: {e}"))?;
        let (lat0, lon0) = points.next().context("grid has no points")?;
        let (_, lon1) = points.next().context("grid has one point")?;
        // Index `nx` is the first cell of row 1; two points are already consumed.
        let (lat_row1, _) = points.nth(nx - 2).context("grid has one row")?;
        let decoder = grib::Grib2SubmessageDecoder::from(sub)
            .map_err(|e| anyhow!("creating decoder: {e}"))?;
        let values: Vec<u8> = decoder
            .dispatch()
            .map_err(|e| anyhow!("decoding values: {e}"))?
            .map(encode_dbz)
            .collect();
        if values.len() != nx * ny {
            bail!("expected {} values, got {}", nx * ny, values.len());
        }
        Ok(Grid { nx, ny, lat0, lon0, dlat: lat0 - lat_row1, dlon: lon1 - lon0, values })
    }

    /// Packed value of the cell nearest (lat, lon); `None` outside the grid.
    pub fn cell(&self, lat: f64, lon: f64) -> Option<u8> {
        let row = ((self.lat0 - lat) / self.dlat).round();
        let col = ((lon - self.lon0) / self.dlon).round();
        if row < 0.0 || col < 0.0 || row >= self.ny as f64 || col >= self.nx as f64 {
            return None;
        }
        Some(self.values[row as usize * self.nx + col as usize])
    }

    /// Strongest packed value among cells whose centres fall in the box (see `strength`).
    /// `None` when the box misses the grid entirely.
    pub fn strongest_in(&self, north: f64, south: f64, west: f64, east: f64) -> Option<u8> {
        let first_row = ((self.lat0 - north) / self.dlat).ceil().max(0.0);
        let last_row = ((self.lat0 - south) / self.dlat).floor().min(self.ny as f64 - 1.0);
        let first_col = ((west - self.lon0) / self.dlon).ceil().max(0.0);
        let last_col = ((east - self.lon0) / self.dlon).floor().min(self.nx as f64 - 1.0);
        if first_row > last_row || first_col > last_col {
            return None;
        }
        let (cols, rows) = (first_col as usize..=last_col as usize, first_row as usize..=last_row as usize);
        rows.flat_map(|r| self.values[r * self.nx..][cols.clone()].iter().copied())
            .max_by_key(|&v| strength(v))
    }

    pub fn bounds(&self) -> (f64, f64, f64, f64) {
        let north = self.lat0 + self.dlat / 2.0;
        let south = self.lat0 - (self.ny as f64 - 0.5) * self.dlat;
        let west = self.lon0 - self.dlon / 2.0;
        let east = self.lon0 + (self.nx as f64 - 0.5) * self.dlon;
        (south, west, north, east)
    }

    pub fn cell_center(&self, index: usize) -> (f64, f64) {
        let (row, col) = (index / self.nx, index % self.nx);
        (self.lat0 - row as f64 * self.dlat, self.lon0 + col as f64 * self.dlon)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 3x2 grid: centres at lat 30/29, lon -90/-89/-88.
    fn tiny() -> Grid {
        Grid {
            nx: 3,
            ny: 2,
            lat0: 30.0,
            lon0: -90.0,
            dlat: 1.0,
            dlon: 1.0,
            values: vec![
                encode_dbz(10.0), encode_dbz(20.0), NO_COVERAGE,
                encode_dbz(30.0), NO_DATA, encode_dbz(50.0),
            ],
        }
    }

    #[test]
    fn encode_decode_roundtrip() {
        assert_eq!(decode_dbz(encode_dbz(58.5)), Some(58.5));
        assert_eq!(decode_dbz(encode_dbz(0.0)), Some(0.0));
        assert_eq!(decode_dbz(encode_dbz(500.0)), Some(126.5));
        assert_eq!(encode_dbz(-99.0), NO_DATA, "-99 is clear air inside coverage");
        assert_eq!(encode_dbz(-5.0), NO_DATA);
        assert_eq!(encode_dbz(f32::NAN), NO_DATA);
        assert_eq!(encode_dbz(-999.0), NO_COVERAGE, "-999 is beyond radar coverage");
        assert_eq!(decode_dbz(NO_COVERAGE), None);
    }

    #[test]
    fn cell_picks_nearest_and_is_none_outside() {
        let g = tiny();
        assert_eq!(g.cell(30.2, -89.9), Some(encode_dbz(10.0)));
        assert_eq!(g.cell(29.0, -88.0), Some(encode_dbz(50.0)));
        assert_eq!(g.cell(30.0, -88.0), Some(NO_COVERAGE));
        assert_eq!(g.cell(30.6, -90.0), None);
        assert_eq!(g.cell(28.4, -90.0), None);
        assert_eq!(g.cell(30.0, -90.6), None);
        assert_eq!(g.cell(30.0, -87.4), None);
    }

    #[test]
    fn strongest_prefers_echo_then_clear_then_no_coverage() {
        let g = tiny();
        // Whole grid: strongest echo wins.
        assert_eq!(g.strongest_in(30.5, 28.5, -90.5, -87.5), Some(encode_dbz(50.0)));
        // Only the no-coverage cell and the clear cell (column -88 top, column -89 bottom).
        let only = Grid { values: vec![NO_COVERAGE, NO_DATA, NO_COVERAGE, NO_COVERAGE, NO_COVERAGE, NO_COVERAGE], ..tiny() };
        assert_eq!(only.strongest_in(30.5, 28.5, -90.5, -87.5), Some(NO_DATA));
        let none = Grid { values: vec![NO_COVERAGE; 6], ..tiny() };
        assert_eq!(none.strongest_in(30.5, 28.5, -90.5, -87.5), Some(NO_COVERAGE));
        assert_eq!(g.strongest_in(40.0, 35.0, -90.5, -87.5), None, "block entirely outside");
    }

    #[test]
    fn bounds_and_cell_centres() {
        let g = tiny();
        assert_eq!(g.bounds(), (28.5, -90.5, 30.5, -87.5));
        assert_eq!(g.cell_center(4), (29.0, -89.0));
    }
}
